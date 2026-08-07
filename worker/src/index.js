/**
 * The model, and nothing else.
 *
 * This is one endpoint: hand it a conversation and the tools the caller can
 * run, and it puts them to whichever free model is answering fastest right now
 * and hands back either the reply or the tool the model wants used. It does not
 * know what ELDEN RING is beyond a system prompt, and it never touches a wiki.
 *
 * The tools are implemented in the launcher because what they reach is there:
 * both wikis are mirrored onto the player's own machine, so searching them
 * costs nothing and only the passages the model actually asked for ever cross
 * the network. Small requests are the only reason a set of free tiers stretches
 * across a whole userbase.
 *
 * No key ever ships in the launcher. They live here as secrets, and the
 * launcher only knows this URL.
 *
 * ── Why a pool rather than a provider ────────────────────────────────────────
 *
 * Any one free tier will fail on somebody: a daily cap runs out, a provider has
 * a bad afternoon, a model is quietly retired. So there is no primary. There is
 * a set of lanes — one per (provider, model) — and each request goes to
 * whichever lane currently looks best, measured rather than assumed:
 *
 *   * every lane carries a rolling average of how long it has been taking, and
 *     the fastest healthy one goes first;
 *   * a lane that fails is put in a cooldown that doubles each time, so a
 *     provider having an outage is stepped around instead of hammered;
 *   * every lane has a daily budget, counted down as it is used, so it is
 *     retired for the day *before* it starts returning 429s;
 *   * and if the leader is slow to come back, the runner-up is started
 *     alongside it and whichever answers first wins.
 *
 * That last one is what makes it feel like nothing ever stalls. It costs a
 * second request occasionally, out of an allowance of sixteen thousand.
 *
 * State lives in the isolate rather than in KV. Cloudflare's free KV allows a
 * thousand writes a day, which a per-request counter would burn through by
 * lunchtime; an isolate handles many requests and losing its memory only costs
 * one wasted retry, so in-memory is both cheaper and better.
 */

/** Bumped by hand on each deploy, so `/health` can prove which code is live. */
const BUILD = "2026-08-07.22";

/**
 * One lane per (provider, model). Every one of these was timed from a
 * Cloudflare datacentre — the numbers a laptop sees are different and
 * irrelevant, because this is where the calls are made from.
 *
 * `daily` is the provider's published free allowance. `weight` is how much the
 * answer is worth: a 70B answers a "how do I beat this boss" question better
 * than an 8B, so it is preferred while it is anywhere near as quick.
 */
const LANES = [
  // Groq: 73 ms from here, and 14,400 a day. The backbone.
  { id: "groq/llama-3.3-70b", secret: "GROQ_KEY", provider: "groq", weight: 1.0, daily: 13000,
    url: "https://api.groq.com/openai/v1/chat/completions", model: "llama-3.3-70b-versatile" },
  { id: "groq/gpt-oss-120b", secret: "GROQ_KEY", provider: "groq", weight: 1.0, daily: 1000,
    url: "https://api.groq.com/openai/v1/chat/completions", model: "openai/gpt-oss-120b" },

  // Gemini. Google refuses this call from some countries; from here it does
  // not, which is half the reason this service exists.
  { id: "gemini/flash", secret: "GEMINI_KEY", provider: "google", weight: 1.0, daily: 1400,
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
    model: "gemini-flash-latest" },

  { id: "mistral/medium", secret: "MISTRAL_KEY", provider: "mistral", weight: 0.95, daily: 2000,
    url: "https://api.mistral.ai/v1/chat/completions", model: "mistral-medium-latest" },

  // NVIDIA lists 131 models and 64 of them say "free endpoint". Rather fewer
  // than that answer, and the catalogue is not a reliable guide to which:
  //
  //   z-ai/glm-5.2            hangs. Three minutes, no response, no error.
  //   moonshotai/kimi-k2.6    404, listed but not deployed.
  //   qwen/qwen3-235b-a22b    404. It was in this file for an afternoon on the
  //                           strength of the listing, which is the mistake
  //                           this comment exists to stop somebody repeating.
  //   deepseek-v4-flash       410 Gone, retired at 09:00 on the day it broke.
  //   meta/llama-3.1-8b       answered, and repeated "彼女の攻撃を回避し" five
  //                           times in one Japanese sentence.
  //
  // These two answer, hold a language, and call tools. Nothing goes in here
  // that has not done all three.
  //
  // Nemotron Ultra is the best thing in the pool: 550B, a million tokens of
  // context, and built for exactly this — it reads a question in Russian and
  // writes `{"query":"Radahn boss guide strategy how to beat"}` in three
  // seconds.
  //
  // Its thinking is switched off, which is worth six seconds a question: the
  // same question answered in 18.9s with it on and 3.2s with it off. The
  // trade is real and it is the right way round here — thinking helps a model
  // reason from its own memory, and this one is answering out of wiki passages
  // it has just been handed. Where there is nothing to hand it, the tool rounds
  // have already established that, and a slower guess is not a better one.
  { id: "nvidia/nemotron-ultra", secret: "NVIDIA_KEY", provider: "nvidia", weight: 1.0, daily: 1000,
    url: "https://integrate.api.nvidia.com/v1/chat/completions",
    model: "nvidia/nemotron-3-ultra-550b-a55b",
    extra: { chat_template_kwargs: { enable_thinking: false } } },
  { id: "nvidia/llama-3.3-70b", secret: "NVIDIA_KEY", provider: "nvidia", weight: 0.95, daily: 1000,
    url: "https://integrate.api.nvidia.com/v1/chat/completions",
    model: "meta/llama-3.3-70b-instruct" },
  { id: "nvidia/minimax-m3", secret: "NVIDIA_KEY", provider: "nvidia", weight: 0.9, daily: 1000,
    url: "https://integrate.api.nvidia.com/v1/chat/completions", model: "minimaxai/minimax-m3" },

  { id: "openrouter/nemotron", secret: "OPENROUTER_KEY", provider: "openrouter", weight: 0.9, daily: 45,
    url: "https://openrouter.ai/api/v1/chat/completions",
    model: "nvidia/nemotron-3-super-120b-a12b:free" },

  // Cloudflare's own, reached through the binding. The one lane that cannot be
  // rate-limited away by somebody else's outage — and the one nothing else
  // should ever lose a race to.
  //
  // It answered a Japanese question in English, invented three spirit ashes
  // that do not exist, and turned "how do I get more flasks" into a page of
  // headings. A reasoning model also spends most of its budget thinking, so it
  // is the one that gets cut off mid-sentence. The weight is what it is because
  // this is the lane of last resort: better than nothing, worse than anything.
  { id: "cloudflare/qwen3-30b", edge: true, provider: "cloudflare", weight: 0.12, daily: 500,
    model: "@cf/qwen/qwen3-30b-a3b-fp8" },
];

/**
 * Lanes that turned out not to do tool calling.
 *
 * Discovered rather than declared: providers disagree about which of their
 * models take a `tools` array and the documentation is behind the deployments.
 * A refusal here is not a fault — the lane is still used for the answer itself,
 * which is most of the work.
 */
const toolless = new Set();

/**
 * How long a lane sits out after failing, doubling each time.
 *
 * Short, because the usual failure is a token-per-minute limit rather than an
 * outage — the account is over its allowance for a few seconds and then it is
 * not. Twenty seconds was long enough that a burst of questions retired the
 * whole pool and it stayed retired.
 */
const COOLDOWN_MS = 6_000;
const COOLDOWN_MAX_MS = 3 * 60_000;
/** If the leader has not answered by now, start the runner-up alongside it. */
const HEDGE_AFTER_MS = 1_200;
/** Give up after this many lanes rather than walking the whole list. */
const MAX_LANES = 4;

/**
 * Per-lane health, for the life of this isolate.
 *
 * `ema` starts unset and is filled by the first success, so the ordering is
 * measured from the very first request rather than seeded with a guess that
 * might be wrong for this datacentre.
 */
const health = new Map();

function laneState(id) {
  let state = health.get(id);
  if (!state) {
    state = { ema: null, fails: 0, coolUntil: 0, used: 0, day: today(), ok: 0 };
    health.set(id, state);
  }
  // Allowances reset at midnight UTC, so the counter has to as well.
  const now = today();
  if (state.day !== now) {
    state.day = now;
    state.used = 0;
  }
  return state;
}

const today = () => Math.floor(Date.now() / 86_400_000);

function succeeded(id, ms) {
  const state = laneState(id);
  // A rolling average, weighted towards recent calls so a provider that has
  // gone slow is demoted within a handful of requests rather than eventually.
  state.ema = state.ema === null ? ms : state.ema * 0.7 + ms * 0.3;
  state.fails = 0;
  state.coolUntil = 0;
  state.used += 1;
  state.ok += 1;
}

function failed(id, exhausted) {
  const state = laneState(id);
  state.fails += 1;
  state.used += 1;
  // A lane that says it is out of allowance is out for the day, not for a
  // minute — retrying it is pure latency for everybody behind you.
  state.coolUntil = exhausted
    ? Date.now() + 24 * 3600_000
    : Date.now() + Math.min(COOLDOWN_MS * 2 ** (state.fails - 1), COOLDOWN_MAX_MS);
}

/**
 * Every lane, once per key it can be run with.
 *
 * A rate limit belongs to an account. Two people who each have a free Gemini
 * key have two allowances, not one, and there is no reason the pool should only
 * know about the first of them.
 *
 * So a second key is a secret named `GEMINI_KEY_2`, and nothing else: this
 * finds it, and the lane it makes carries its own provider name so a 429 on one
 * account never cools the other. Adding a third is adding `GEMINI_KEY_3`. There
 * is no code to change and no list to keep in step, which matters because the
 * person adding the key is not going to be the person editing this file.
 *
 * These are separate accounts belonging to separate people who offered them.
 * The pool does not want more keys on one account — that is against every
 * provider's terms, and a ban would take out the whole pool at once rather than
 * one lane of it.
 */
function withKeys(env) {
  const out = [];
  for (const lane of LANES) {
    if (lane.edge) {
      if (env.AI) out.push(lane);
      continue;
    }
    if (env[lane.secret]) out.push(lane);
    // `_2` upward, until one is missing.
    for (let n = 2; n <= 9; n++) {
      const secret = `${lane.secret}_${n}`;
      if (!env[secret]) break;
      out.push({
        ...lane,
        id: `${lane.id}#${n}`,
        secret,
        provider: `${lane.provider}#${n}`,
      });
    }
  }
  return out;
}

/**
 * The lanes worth trying, best first.
 *
 * A lane that has never been tried is given an optimistic estimate so it gets
 * one chance early; after that it is ranked on what it has actually done. The
 * score divides by weight, so a better model wins ties and a much better model
 * wins even when it is somewhat slower.
 */
function order(env, needsTools = false) {
  const now = Date.now();
  return withKeys(env)
    // A lane that has refused a tool call once is not asked again this
    // isolate's lifetime. Providers differ about which models take tools and
    // the only reliable way to find out is to be told no.
    .filter((lane) => !(needsTools && toolless.has(lane.id)))
    .map((lane) => ({ lane, state: laneState(lane.id) }))
    .filter(({ lane, state }) => state.coolUntil <= now && state.used < lane.daily)
    .map((entry) => {
      const { lane, state } = entry;
      const ms = state.ema ?? 400;
      // Nearly out of allowance? Drift down the order so the last of it is
      // saved for when everything else is gone.
      const nearEmpty = state.used > lane.daily * 0.9 ? 3 : 1;
      return { ...entry, score: (ms / lane.weight) * nearEmpty };
    })
    .sort((a, b) => {
      // Speed and quality decide it, as before — but only when they actually
      // differ. Within a tenth of a second the difference is network weather,
      // not a better lane, and sorting on it means the same account takes every
      // request while nine identical ones sit idle. Ten accounts are only ten
      // accounts if the work reaches all of them.
      const gap = Math.abs(a.score - b.score);
      const alike = gap < Math.max(120, Math.min(a.score, b.score) * 0.15);
      if (alike) return a.state.used - b.state.used;
      return a.score - b.score;
    })
    // One lane per provider, and the best of them.
    //
    // A rate limit belongs to a key, not to a model: Groq's two lanes share one
    // account, so a 429 on one is a 429 waiting on the other. Walking down a
    // list that ran groq, groq, nvidia, nvidia meant two of the four attempts
    // were spent asking a provider that had just said no — and with the retries
    // that follow, every lane ended up cooling down at once.
    //
    // Spreading them means four attempts reach four different companies.
    .filter((entry, at, all) => all.findIndex((o) => o.lane.provider === entry.lane.provider) === at)
    .slice(0, MAX_LANES);
}

/**
 * A provider that has just refused, and everything on its key with it.
 *
 * A token-per-minute limit is the whole account's, so the sibling lane is
 * already over it too. Cooling them together stops the next round walking
 * straight into the same wall.
 */
function coolProvider(id, exhausted, env) {
  const all = env ? withKeys(env) : LANES;
  const which = all.find((lane) => lane.id === id)?.provider;
  for (const lane of all) {
    if (lane.provider === which) failed(lane.id, exhausted);
  }
}

/**
 * How the thing talks.
 *
 * Three things were wrong with the prompt this replaces, all of them found by
 * using it. It was told to answer only from the passages, so a question the
 * model knew perfectly well — what a stat does, what a word means — came back
 * as "the passages do not cover it", which reads as stupid because it is. It
 * was told to answer "in the language the question was asked in", which models
 * follow most of the time and drop on short questions. And it had no idea what
 * had been said a moment earlier, so "and how do I beat her?" was a question
 * about nobody.
 *
 * The passages are still the authority where they exist. What changed is that
 * their absence is no longer a reason to say nothing.
 */
function system(edition) {
  const playing = edition
    ? `They are playing ${edition}, a total-conversion mod that rebalances much of the base game. Both wikis are searchable; where the two disagree, the mod's is the one that applies to them.`
    : "They are playing the base game.";

  return `You are the assistant built into Roundtable, a launcher for ELDEN RING. You
are talking to a player who is mid-game, has one hand on the controller, and wants
the answer now. ${playing}

TOOLS
Everything you can reach is on this player's own machine: both wikis, the game's
item database, and their save file. Nothing is a web search.

Reach for them whenever a question turns on something they would know — a boss, an
item, a stat, a location, a quest, a number, a mod change, or where this player
actually is in their game. Search in English with the names the game uses, whatever
language the player wrote in; the wikis are English. If a search finds the wrong
thing, search again with different words.

Pick the right one. A figure — attack, scaling, weight, requirements, drops — is in
the item database and is exact; getting it out of wiki prose is slower and easier to
misread. How to do something, why something happens, what a quest wants: that is the
wiki. Whether a thing suits *them* — worth it at their level, already installed,
possible in their version: that is their save and their setup.

You can ask for several at once, and you should when they do not depend on each
other. Checking an item's numbers and reading its article is one round, not two.

Do not use any of them for a greeting, a question about you or the launcher, an
opinion, or something general about the genre. Just answer.

CHECKING
Search results are titles, not facts. Read the article before stating anything out of
it, and never answer from a title alone — "Radahn Soldier Set" turning up in a search
is not evidence about Radahn.

Check anything specific twice, from two different places. A number, a requirement, a
drop, a scaling letter: put the item database beside the article, or the mod's wiki
beside the base game's. They disagree constantly, because the mod rebalances the base
game — and when they do, say so and give the player the figure for the version they
are actually running. Two sources agreeing needs no comment; two sources disagreeing
is worth a clause.

If what you found does not actually answer what was asked, do not stretch it into an
answer. Search once more with different words. If it still is not there, say so.

LANGUAGE
Reply in the same language as the player's last message. Every word of it, including
anything you took out of an English wiki. Russian gets Russian, Ukrainian gets
Ukrainian, Japanese gets Japanese. Proper nouns keep their English spelling, with the
local name beside them the first time if there is one. Never switch to English
because the source material was English.

ANSWERING
What you looked up outranks anything you remember — the game has been patched many
times and a mod changes numbers outright. Where you could not find it, say plainly
that it is not there, then answer from your own knowledge and mark it as memory.

Memory is for how things work, never for figures. If the player names an item and
the searches came back empty, the honest answer is that it is not in this game —
say that and stop. Do not then supply its damage "from memory": you have just
established you cannot find it, and a number invented in that position is the most
convincing kind of wrong. This has happened: asked about a sword that does not
exist, the answer was that it deals "around 650-700 physical". Nobody was helped.

Never invent a number, an item name or a place. Being vague is fine; being
confidently wrong about a boss costs somebody a run.

Do not describe the looking. "According to the wiki", "based on the passages", "I
searched for" — none of that is the answer. Say the thing. The one exception is when
sources disagree or when you are going from memory, which the player does need told.

Answer the question that was asked and stop. Cover it properly — the thing itself,
the caveat that matters, and the next thing they will need — but nothing beyond
that. Nobody asked for every location of every Golden Seed; they asked how to get
more flasks. Around eighty words is right for most questions, more only when the
question genuinely has several parts. No preamble, no restating the question, no
offering to help further.

Plain text only. What you write is shown as-is in a narrow window over a running
game, so asterisks, hashes and backticks arrive as asterisks, hashes and backticks,
and a table arrives as rubble. Write sentences. If the answer really is a list, use
short lines beginning with "- " and nothing else.`;
}

/** True when the refusal means "your allowance is gone", not "try again". */
function isExhausted(status, body) {
  if (status === 402) return true;
  if (status !== 429) return false;
  return /quota|balance|insufficient|credit|exceeded your/i.test(body);
}

/** Room to answer properly, and room to think before calling a tool. */
const MAX_TOKENS = 1600;

/**
 * One turn, which may come back as an answer or as a tool the model wants run.
 *
 * The tools themselves live in the launcher, because the wikis do — this only
 * carries the request out and the model's decision back.
 */
async function callLane(lane, env, messages, signal, tools) {
  const started = Date.now();

  if (lane.edge) {
    const out = await env.AI.run(lane.model, {
      messages,
      max_tokens: MAX_TOKENS,
      temperature: 0.3,
      ...(tools ? { tools } : {}),
    });
    const calls = normaliseCalls(out.tool_calls);
    const text = (out.response ?? "").trim();
    if (!text && calls.length === 0) {
      throw Object.assign(new Error("empty answer"), { lane: lane.id });
    }
    return { text, calls, ms: Date.now() - started, lane: lane.id };
  }

  const res = await fetch(lane.url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${env[lane.secret]}` },
    body: JSON.stringify({
      model: lane.model,
      messages,
      max_tokens: MAX_TOKENS,
      temperature: 0.3,
      ...(lane.extra ?? {}),
      ...(tools ? { tools, tool_choice: "auto" } : {}),
    }),
    signal,
  });

  if (!res.ok) {
    const body = (await res.text()).slice(0, 200);
    throw Object.assign(new Error(`${res.status} ${body}`), {
      lane: lane.id,
      exhausted: isExhausted(res.status, body),
    });
  }

  const message = (await res.json()).choices?.[0]?.message ?? {};
  const calls = normaliseCalls(message.tool_calls);
  // The answer, and only the working when there is no answer — a reasoning
  // model can leave `content` empty and put everything in the other field,
  // which looks like success and reads as silence.
  const text = (message.content || message.reasoning_content || "").trim();
  if (!text && calls.length === 0) {
    throw Object.assign(new Error("empty answer"), { lane: lane.id });
  }
  return { text, calls, ms: Date.now() - started, lane: lane.id };
}

/**
 * Tool calls in the one shape the launcher reads.
 *
 * Providers disagree about the details — Workers AI returns `{name, arguments}`
 * with the arguments already parsed, everyone else returns the OpenAI shape
 * with them as a JSON string, and some omit the id. All of it is flattened here
 * so the launcher never has to know which lane answered.
 */
function normaliseCalls(calls) {
  if (!Array.isArray(calls)) return [];
  return calls
    .map((call, index) => {
      const fn = call.function ?? call;
      const args = fn.arguments ?? call.arguments ?? {};
      return {
        id: call.id ?? `call_${index}`,
        type: "function",
        function: {
          name: fn.name ?? "",
          arguments: typeof args === "string" ? args : JSON.stringify(args),
        },
      };
    })
    .filter((call) => call.function.name);
}

/**
 * The same call, delivered a token at a time.
 *
 * Waiting for a whole answer and waiting for the first word of one are
 * different experiences of the same second and a half. Every lane here speaks
 * the OpenAI streaming shape, so this is the same request with `stream` set and
 * a reader over the response.
 *
 * Yields strings. The first one is what matters: it is the moment the thing
 * stops looking stuck, and it is what the hedge below races on.
 */
async function* streamLane(lane, env, messages, signal) {
  if (lane.edge) {
    // Workers AI streams too, in the same event shape.
    const out = await env.AI.run(lane.model, {
      messages,
      max_tokens: MAX_TOKENS,
      temperature: 0.3,
      stream: true,
    });
    for await (const piece of readEvents(out, (data) =>
      data.response ? { answer: data.response } : null,
    )) {
      yield piece.answer;
    }
    return;
  }

  const res = await fetch(lane.url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${env[lane.secret]}` },
    body: JSON.stringify({
      model: lane.model,
      messages,
      max_tokens: MAX_TOKENS,
      temperature: 0.3,
      stream: true,
      ...(lane.extra ?? {}),
    }),
    signal,
  });

  if (!res.ok) {
    const body = (await res.text()).slice(0, 200);
    throw Object.assign(new Error(`${res.status} ${body}`), {
      lane: lane.id,
      exhausted: isExhausted(res.status, body),
    });
  }

  // Reasoning and answer, kept apart.
  //
  // A reasoning model sends its working in `reasoning_content` and its answer
  // in `content`, and the two interleave: the first dozen events of a Nemotron
  // reply are pure reasoning. Taking whichever field a given event happens to
  // carry — which is what `content ?? reasoning_content` does — put both on
  // screen, so an answer about Radahn opened with "The user is asking how to
  // kill Radahn in Russian. I found information from both wikis…".
  //
  // But the fallback cannot simply go, because other models put the whole
  // answer in `reasoning_content` and leave `content` empty, and dropping it
  // there means silence. So the working is held to one side and only used if
  // nothing better ever arrives.
  let thinking = "";
  let answered = false;

  for await (const piece of readEvents(res.body, (data) => {
    const delta = data.choices?.[0]?.delta ?? {};
    if (delta.content) return { answer: delta.content };
    if (delta.reasoning_content) return { working: delta.reasoning_content };
    return null;
  })) {
    if (piece.answer) {
      answered = true;
      thinking = "";
      yield piece.answer;
      continue;
    }
    if (!answered) thinking += piece.working;
  }

  // Nothing but working: it was the answer after all.
  if (!answered && thinking.trim()) yield thinking;
}

/**
 * Server-sent events off a byte stream, as the text they carry.
 *
 * Every provider here speaks the same event shape and none of them respects
 * message boundaries: an event arrives split across two reads as often as not,
 * so only whole lines are taken and the remainder is carried forward.
 */
async function* readEvents(body, pick) {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      let cut;
      while ((cut = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, cut).trim();
        buffer = buffer.slice(cut + 1);
        if (!line.startsWith("data:")) continue;
        const payload = line.slice(5).trim();
        if (payload === "[DONE]") return;
        try {
          const picked = pick(JSON.parse(payload));
          if (picked) yield picked;
        } catch {
          // A malformed event is one dropped token, not a failed answer.
        }
      }
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

/**
 * Everything the player should not see, taken out of the stream.
 *
 * Three things leak out of these models, and all three arrive mixed into the
 * answer a few characters at a time:
 *
 *   * reasoning. Some wrap it in `<think>`, some just lead with a blank line
 *     where it used to be. It arrives *before* the answer, so on a stream it is
 *     the first thing the player sees — or, with the blank line, nothing at all
 *     for several seconds.
 *   * tool calls written as prose. On the round where the tools are taken away,
 *     a model that still wants one writes it out instead:
 *     `<function=read_article>{"title":…}</function>` arrived as a whole answer
 *     once. That is worse than a wrong answer, because it is not in a language.
 *   * markdown. The overlay is a narrow window over a game and draws text as it
 *     is given, so asterisks arrive as asterisks. The models are told not to;
 *     the large ones comply and the small ones produce `### Limgrave` and
 *     three-column tables regardless.
 *
 * None of it can be done by running a regular expression over each piece,
 * because a piece is whatever the network handed over: `<thi` in one and `nk>`
 * in the next, or `**bold` two hundred characters before the closing `**`. The
 * first attempt at this did exactly that and mangled every case when the pieces
 * were small — `**Bold**` came out as `**Bold**`, and a `>` that happened to
 * begin a piece was eaten as a block quote.
 *
 * So text is released only up to the last point where nothing is still open.
 * Plain prose has nothing open and streams straight through, which is the
 * common case; inside an emphasis span or an unfinished tag it waits, which
 * lasts a word.
 */
function clean() {
  /** Seen but not released, because something in it is still unresolved. */
  let held = "";
  /** Waiting for this to close, having entered something to be swallowed. */
  let swallowing = null;
  /** Whether what comes next begins a line, which the line rules need to know. */
  let atLineStart = true;
  /** Nothing has been released yet, so leading blanks can still be dropped. */
  let opened = false;

  /** Openings worth waiting for, and what closes each. */
  const traps = [
    { open: "<think>", close: "</think>" },
    { open: "<function=", close: "</function>" },
    { open: "<tool_call>", close: "</tool_call>" },
    { open: "<|python_tag|>", close: "\n" },
  ];

  /** How far into `held` everything is settled. */
  function settled() {
    let at = 0;
    let lineStart = atLineStart;

    while (at < held.length) {
      const ch = held[at];

      // A line whose first characters could make it a heading, a quote, a
      // bullet or a table row is not settled until the line ends: the rules
      // for those need the whole line, and `#` on its own is just a character.
      if (lineStart) {
        const rest = held.slice(at);
        const marker = /^[ \t]*(#{1,6}|>|[-*+]|\||\d+\.|[-_*]{3,})/.exec(rest);
        if (marker) {
          const ends = held.indexOf("\n", at);
          if (ends === -1) return at;
          at = ends + 1;
          continue;
        }
      }
      lineStart = ch === "\n";

      if (ch === "<") {
        const rest = held.slice(at);
        // A whole opener: handled by the caller, which enters swallow mode.
        if (traps.some((t) => rest.toLowerCase().startsWith(t.open.toLowerCase()))) return at;
        // Might still become one, given more characters.
        if (traps.some((t) => t.open.toLowerCase().startsWith(rest.toLowerCase()))) return at;
        at += 1;
        continue;
      }

      // An emphasis or code run, which means nothing until it closes.
      const run = /^(\*{1,3}|`{1,3})/.exec(held.slice(at));
      if (run) {
        const marker = run[1];
        const close = held.indexOf(marker, at + marker.length);
        if (close === -1) return at;
        at = close + marker.length;
        continue;
      }

      // A link, which is only a link once its target arrives.
      if (ch === "[") {
        const shut = held.indexOf(")", at);
        if (shut === -1) return at;
        at = shut + 1;
        continue;
      }

      at += 1;
    }
    return held.length;
  }

  /**
   * A settled span as plain text.
   *
   * `begins` says whether this span starts at a real line start. Without it the
   * line rules fire against the start of a *fragment*, which is how "x > y"
   * lost its greater-than sign.
   */
  function render(span, begins) {
    if (begins) return plain(span);
    const cut = span.indexOf("\n");
    if (cut === -1) return inline(span);
    return inline(span.slice(0, cut)) + plain(span.slice(cut));
  }

  function release(span) {
    if (!span) return "";
    let out = render(span, atLineStart);
    atLineStart = span.endsWith("\n");
    if (!opened) {
      out = out.replace(/^\s+/, "");
      if (out) opened = true;
    }
    return out;
  }

  return {
    /** What is safe to show now, which may be nothing yet. */
    take(piece) {
      held += piece;
      let out = "";

      for (;;) {
        if (swallowing) {
          const at = held.toLowerCase().indexOf(swallowing.toLowerCase());
          if (at === -1) {
            // A model that never closes what it opened must not silence the
            // whole answer.
            if (held.length > 8000) {
              swallowing = null;
              continue;
            }
            return out;
          }
          held = held.slice(at + swallowing.length);
          swallowing = null;
          continue;
        }

        const safe = settled();
        out += release(held.slice(0, safe));
        held = held.slice(safe);

        const trap = traps.find((t) => held.toLowerCase().startsWith(t.open.toLowerCase()));
        if (trap) {
          held = held.slice(trap.open.length);
          swallowing = trap.close;
          continue;
        }

        // Something unresolved is at the front. Wait for more — unless it has
        // been waiting far too long, in which case it was never markdown.
        if (held.length > 2000) {
          out += release(held);
          held = "";
          continue;
        }
        return out;
      }
    },

    /** Whatever is still held, now that nothing more is coming. */
    rest() {
      const out = swallowing ? "" : release(held);
      held = "";
      swallowing = null;
      return out;
    },
  };
}

/** Emphasis, code and links, which can appear anywhere in a line. */
function inline(text) {
  return text
    .replace(/\*\*\*(.+?)\*\*\*/gs, "$1")
    .replace(/\*\*(.+?)\*\*/gs, "$1")
    .replace(/(^|[^*])\*([^*\n]+?)\*(?!\*)/g, "$1$2")
    .replace(/`{1,3}([^`]+?)`{1,3}/gs, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1");
}

/** The same, plus the constructs that only mean anything at a line start. */
function plain(text) {
  return inline(text)
    // Headings and quotes become ordinary lines.
    .replace(/^[ \t]*#{1,6}[ \t]+/gm, "")
    .replace(/^[ \t]*>[ \t]?/gm, "")
    // Bullets keep their shape, since a list is sometimes the right answer.
    .replace(/^[ \t]*[*+][ \t]+/gm, "- ")
    // A table row, flattened. The overlay is 340 pixels wide; a table is not
    // going to happen there.
    .replace(/^[ \t]*\|(.+)\|[ \t]*$/gm, (_, row) =>
      /^[\s|:-]+$/.test(row) ? "" : row.split("|").map((c) => c.trim()).filter(Boolean).join(" — "),
    )
    .replace(/^[ \t]*[-_*]{3,}[ \t]*$/gm, "")
    // Three blank lines are two too many in a window this size.
    .replace(/\n{3,}/g, "\n\n");
}

/**
 * The pool, streaming, hedged on time-to-first-token.
 *
 * The leader is started. If it has not produced a word by `HEDGE_AFTER_MS` the
 * runner-up is started beside it, and whichever speaks first wins outright —
 * the other is abandoned mid-sentence. That is the right race for a stream: a
 * lane that is quick to start is quick all the way through, and a lane that is
 * still thinking has already lost.
 *
 * Once a winner has spoken it is committed to. Switching lanes halfway would
 * mean two different models writing one paragraph.
 */
async function* askPoolStreaming(env, messages, deadlineMs = 25_000) {
  const lanes = order(env, false);
  if (lanes.length === 0) {
    yield { error: "no lane has any allowance left" };
    return;
  }

  const started = Date.now();
  const controllers = new Map();
  /** Lanes racing for the first token: id -> { iterator, pending } */
  const racing = new Map();
  let next = 0;

  const begin = () => {
    if (next >= lanes.length) return false;
    const { lane } = lanes[next++];
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), deadlineMs);
    controllers.set(lane.id, { controller, timer });
    const iterator = streamLane(lane, env, messages, controller.signal)[Symbol.asyncIterator]();
    racing.set(lane.id, {
      iterator,
      pending: iterator.next().then(
        (step) => ({ id: lane.id, step }),
        (error) => ({ id: lane.id, error }),
      ),
    });
    return true;
  };

  const stop = (id) => {
    const held = controllers.get(id);
    if (held) {
      clearTimeout(held.timer);
      held.controller.abort();
      controllers.delete(id);
    }
    racing.delete(id);
  };

  /**
   * How long to wait before starting the next lane.
   *
   * Not a fixed delay. The race is decided on who speaks first, and the worst
   * model in the pool is the one physically closest — Cloudflare's own runs in
   * this datacentre, so it produces a first token before anything reachable
   * over the internet can. It kept winning, and then answering a Russian
   * question in English with three spirit ashes that do not exist.
   *
   * So a lane worth less has to wait longer in proportion. The lane of last
   * resort waits about eight times as long as the leader, which is what "last
   * resort" should have meant all along: it is started when nothing else has
   * managed anything, not alongside them.
   */
  const patience = () => {
    const leader = lanes[0]?.lane.weight ?? 1;
    const nextUp = lanes[next]?.lane.weight ?? leader;
    return Math.round(HEDGE_AFTER_MS * Math.max(1, leader / Math.max(nextUp, 0.05)));
  };

  begin();
  let winner = null;
  let first = "";
  // Why each lane declined, kept so a total failure can say what happened
  // rather than "every lane refused" — which is true and useless.
  const tried = [];

  while (racing.size > 0 && !winner) {
    const hedge = new Promise((resolve) => setTimeout(() => resolve("hedge"), patience()));
    const settled = await Promise.race([...[...racing.values()].map((r) => r.pending), hedge]);

    if (settled === "hedge") {
      // Nobody has spoken yet. Another lane costs one request out of thousands
      // and takes a second off the wait when the leader is having a bad moment.
      if (!begin()) {
        const done = await Promise.race([...racing.values()].map((r) => r.pending));
        if (done.error || done.step.done) {
          const why = done.error ?? new Error("produced nothing");
          tried.push({ lane: done.id, why: String(why.message ?? why).slice(0, 160) });
          tallyFailure(done.id, why, env);
          stop(done.id);
          continue;
        }
        winner = done.id;
        first = done.step.value;
      }
      continue;
    }

    if (settled.error || settled.step.done) {
      const why = settled.error ?? new Error("produced nothing");
      tried.push({ lane: settled.id, why: String(why.message ?? why).slice(0, 160) });
      tallyFailure(settled.id, settled.error, env);
      stop(settled.id);
      begin();
      continue;
    }

    winner = settled.id;
    first = settled.step.value;
  }

  if (!winner) {
    yield { error: "every lane refused", tried };
    return;
  }

  // The losers are abandoned the moment there is a winner.
  for (const id of [...racing.keys()]) if (id !== winner) stop(id);

  const held = racing.get(winner);
  const filter = clean();

  let whole = "";
  const push = (piece) => {
    const shown = filter.take(piece);
    whole += shown;
    return shown;
  };

  const opening = push(first);
  yield { lane: winner, delta: opening };

  try {
    for (;;) {
      const step = await held.iterator.next();
      if (step.done) break;
      const shown = push(step.value);
      if (shown) yield { delta: shown };
    }
  } catch {
    // A stream that dies partway still leaves the player with what arrived.
  }
  // Whatever the filter was still holding back, now that nothing more is
  // coming — the last word of an answer is usually in here.
  const tail = filter.rest();
  if (tail) {
    whole += tail;
    yield { delta: tail };
  }
  stop(winner);

  const ms = Date.now() - started;
  if (whole.trim()) succeeded(winner, ms);
  yield { done: true, lane: winner, ms };
}

function tallyFailure(id, error, env) {
  coolProvider(id, Boolean(error?.exhausted), env);
}

/**
 * Asks the pool, hedging.
 *
 * The leader is started; if it has not come back within `HEDGE_AFTER_MS` the
 * next lane is started too and both are left running. Whichever answers first
 * wins and the loser is abandoned. On a failure the next unstarted lane takes
 * its place immediately, so an outage costs the time of one timeout rather than
 * one timeout per provider.
 */
async function askPool(env, messages, deadlineMs = 25_000, tools = null) {
  const lanes = order(env, Boolean(tools));
  if (lanes.length === 0) return { error: "no lane has any allowance left", tried: [] };

  const tried = [];
  const running = new Map();
  const controllers = new Map();
  let next = 0;

  const start = () => {
    if (next >= lanes.length) return false;
    const { lane } = lanes[next++];
    const controller = new AbortController();
    controllers.set(lane.id, controller);
    const timer = setTimeout(() => controller.abort(), deadlineMs);
    running.set(
      lane.id,
      callLane(lane, env, messages, controller.signal, tools)
        .then((out) => ({ out }))
        .catch((error) => ({ error }))
        .finally(() => clearTimeout(timer)),
    );
    return true;
  };

  start();

  while (running.size > 0) {
    const hedge = new Promise((resolve) => setTimeout(() => resolve("hedge"), HEDGE_AFTER_MS));
    const settled = await Promise.race([...running.values(), hedge]);

    if (settled === "hedge") {
      // The leader is taking its time. Run the next one beside it rather than
      // waiting to find out.
      if (!start()) {
        const first = await Promise.race(running.values());
        if (first.out) {
          succeeded(first.out.lane, first.out.ms);
          for (const [, c] of controllers) c.abort();
          return { ...first.out, tried };
        }
        tried.push({ lane: first.error.lane, why: first.error.message?.slice(0, 120) });
        coolProvider(first.error.lane, first.error.exhausted, env);
        running.delete(first.error.lane);
      }
      continue;
    }

    if (settled.out) {
      succeeded(settled.out.lane, settled.out.ms);
      // Whoever else is still going is no longer needed.
      for (const [id, c] of controllers) if (id !== settled.out.lane) c.abort();
      return { ...settled.out, tried };
    }

    const { lane, message, exhausted } = settled.error;
    tried.push({ lane, why: message?.slice(0, 120) });
    if (tools && /tool|function.call/i.test(message ?? "")) {
      // Not a broken lane — one that does not do tool calling. It is still
      // wanted for the answer, so it is remembered rather than cooled down.
      toolless.add(lane);
    } else {
      coolProvider(lane, exhausted, env);
    }
    running.delete(lane);
    start();
  }

  return { error: "every lane refused", tried };
}

/**
 * Keeping the allowance for the people it is for.
 *
 * The keys themselves cannot be taken: they are Worker secrets, they are never
 * sent to a client, and they cannot be read back out — not even by this code.
 * What *can* be taken is the allowance, because anybody who learns this URL can
 * spend a day of everyone's questions on nothing. So the endpoint is what needs
 * defending, not the keys.
 *
 * This is done with Cloudflare's rate limiting binding rather than a counter of
 * our own. A counter in the isolate does not work and it is worth saying why:
 * traffic is spread across isolates and each one starts from zero, so sixteen
 * calls in a row went straight through a cap of twelve when it was tried. The
 * binding is backed by shared state in the datacentre and is not fooled that
 * way.
 */
async function allowed(env, who, fresh) {
  // Only a new question counts against the burst window.
  //
  // This is the whole reason the limit was wrong. It used to count requests,
  // and a question used to be one request — now the model looks things up, so
  // one question is a search, two reads and an answer. Counting those as four
  // questions meant nine real questions in a row exhausted an allowance meant
  // for far more, and the tenth was refused. What needs limiting is somebody
  // asking a hundred questions, not a model doing its job on one of them.
  if (fresh) {
    const burst = await env.PER_MINUTE?.limit({ key: who });
    if (burst && !burst.success) return { ok: false, retryAfter: 60, why: "too many at once" };
  }

  // The long window still counts everything, because that is the one that
  // protects the day's allowance and every call spends some of it.
  const steady = await env.SUSTAINED?.limit({ key: `hour:${who}` });
  if (steady && !steady.success) return { ok: false, retryAfter: 600, why: "that is enough for now" };

  return { ok: true };
}

/** Constant-time compare, so a wrong token cannot be found one byte at a time. */
function sameSecret(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "content-type, x-admin",
  "access-control-allow-methods": "POST, GET, OPTIONS",
};

const json = (body, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...CORS },
  });

export default {
  async fetch(request, env) {
    if (request.method === "OPTIONS") return new Response(null, { headers: CORS });
    const url = new URL(request.url);

    // What the pool looks like right now: which lanes are up, how quick each
    // has been, and how much of each allowance is left.
    if (url.pathname === "/health") {
      const now = Date.now();
      const lanes = withKeys(env).map((lane) => {
        const state = laneState(lane.id);
        return {
          lane: lane.id,
          ready: state.coolUntil <= now && state.used < lane.daily,
          ms: state.ema === null ? null : Math.round(state.ema),
          used: state.used,
          daily: lane.daily,
          fails: state.fails,
          coolFor: state.coolUntil > now ? Math.round((state.coolUntil - now) / 1000) : 0,
        };
      });
      const budget = lanes.filter((l) => l.ready).reduce((sum, l) => sum + (l.daily - l.used), 0);
      return json({
        ok: lanes.some((l) => l.ready),
        // A stamp, so a stale deployment can be told apart from a broken one.
        // Cloudflare takes about a minute to roll a version out, and without
        // this every propagation delay looks like a bug in the code.
        build: BUILD,
        limiter: { perMinute: Boolean(env.PER_MINUTE), sustained: Boolean(env.SUSTAINED) },
        remainingToday: budget,
        lanes,
      });
    }

    // Every lane, tried once, regardless of health. This is how a lane that has
    // been cooling down gets checked without waiting for it to come back.
    //
    // Behind a token, because one call costs nine requests out of the day's
    // allowance and an open endpoint that does that is a gift to anybody bored.
    if (url.pathname === "/probe" || url.pathname === "/time") {
      if (!env.ADMIN_KEY || !sameSecret(request.headers.get("x-admin") ?? "", env.ADMIN_KEY)) {
        return json({ error: "not for you" }, 403);
      }
    }

    // Twenty calls to the limiter in a row, on one key, so its actual behaviour
    // can be read rather than inferred from whether requests got through.
    if (url.pathname === "/limiter") {
      const key = `debug-${Date.now()}`;
      const out = [];
      for (let i = 0; i < 20; i++) {
        const r = await env.PER_MINUTE.limit({ key });
        out.push(r.success ? 1 : 0);
      }
      return json({ key, results: out.join(""), allowed: out.filter(Boolean).length });
    }

    if (url.pathname === "/probe") {
      const messages = [{ role: "user", content: "Reply with exactly: ok" }];
      const out = await Promise.all(
        withKeys(env).map(async (lane) => {
          try {
            const r = await callLane(lane, env, messages, AbortSignal.timeout(25_000), null);
            succeeded(lane.id, r.ms);
            return { lane: lane.id, ok: true, ms: r.ms, said: r.text.slice(0, 40) };
          } catch (error) {
            return { lane: lane.id, ok: false, why: error.message?.slice(0, 130) };
          }
        }),
      );
      return json(out.sort((a, b) => (a.ms ?? 1e9) - (b.ms ?? 1e9)));
    }

    /*
     * One turn of the conversation.
     *
     * The launcher sends the whole exchange so far and the tools it can run;
     * this adds the system prompt, puts it to the pool, and hands back either
     * the model's words or the tool it wants used. The tools themselves are
     * implemented in the launcher, because what they reach — both wikis — is
     * mirrored on the player's own machine.
     *
     * The endpoints this replaces were a search-term planner and a
     * passage-stuffer: the launcher guessed what a question was about, sent
     * four paragraphs, and the model answered out of them or said it could not.
     * The model does its own looking now, which is the difference between a
     * search box with a model on the end and something that can be asked a
     * question.
     */
    if (url.pathname !== "/chat" || request.method !== "POST") {
      return json({ error: "POST /chat" }, 404);
    }

    // Whether this is somebody asking something, or the model still working on
    // the last thing they asked. Only the first counts as a question.
    let peeked = null;
    try {
      peeked = await request.clone().json();
    } catch {
      /* handled below, where the body is read properly */
    }
    const fresh = !Array.isArray(peeked?.messages) || peeked.messages.length <= 1;

    const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
    const gate = await allowed(env, ip, fresh);
    if (!gate.ok) {
      return new Response(JSON.stringify({ error: gate.why, retryAfter: gate.retryAfter }), {
        status: 429,
        headers: { "content-type": "application/json", "retry-after": String(gate.retryAfter), ...CORS },
      });
    }

    // Room for a conversation and a couple of long wiki sections, and no more.
    // Past that it is either a mistake or somebody using this as a general
    // model, and neither is what the allowance is for.
    const length = Number(request.headers.get("content-length") ?? 0);
    if (length > 160_000) return json({ error: "too much" }, 413);

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "expected JSON" }, 400);
    }

    if (!Array.isArray(body.messages) || body.messages.length === 0) {
      return json({ error: "no messages" }, 400);
    }
    const edition = body.edition ? String(body.edition).slice(0, 60) : null;
    const tools = Array.isArray(body.tools) && body.tools.length > 0 ? body.tools : null;
    const messages = [{ role: "system", content: system(edition) }, ...body.messages.slice(-24)];

    if (!body.stream) {
      const result = await askPool(env, messages, 25_000, tools);
      if (result.error) return json(result, 503);
      return json({
        content: result.text,
        toolCalls: result.calls ?? [],
        lane: result.lane,
        ms: result.ms,
        tried: result.tried,
      });
    }

    // Streamed, as server-sent events. The launcher forwards these on to its
    // own window, so what the player sees is the model typing rather than a
    // spinner and then a paragraph.
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      async start(controller) {
        try {
          for await (const event of askPoolStreaming(env, messages)) {
            controller.enqueue(encoder.encode(`data: ${JSON.stringify(event)}\n\n`));
          }
        } catch (error) {
          const why = String(error?.message ?? error).slice(0, 160);
          controller.enqueue(encoder.encode(`data: ${JSON.stringify({ error: why })}\n\n`));
        }
        controller.enqueue(encoder.encode("data: [DONE]\n\n"));
        controller.close();
      },
    });

    return new Response(stream, {
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        // Cloudflare will buffer a response it thinks it can compress, which
        // turns a stream back into one late blob.
        "content-encoding": "identity",
        ...CORS,
      },
    });
  },
};
