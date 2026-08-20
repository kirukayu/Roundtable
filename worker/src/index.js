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
const BUILD = "2026-08-12.1";

/**
 * One lane per (provider, model). Every one of these was timed from a
 * Cloudflare datacentre — the numbers a laptop sees are different and
 * irrelevant, because this is where the calls are made from.
 *
 * `daily` is the provider's published free allowance. `weight` is how much the
 * answer is worth: a 70B answers a "how do I beat this boss" question better
 * than an 8B, so it is preferred while it is anywhere near as quick.
 *
 * ---------------------------------------------------------------------------
 * What was actually checked, on 11 Aug 2026, and what was not
 * ---------------------------------------------------------------------------
 *
 * `daily` is described above as the published allowance. For NVIDIA it is not:
 * it was a guess, written here as though it were a fact, and it has been
 * quoted back as one. That is the thing to be careful of in this file.
 *
 * Read off the providers' own pages:
 *
 *   NVIDIA        40 requests a minute. NO daily figure is published anywhere,
 *                 and `docs.api.nvidia.com` does not mention limits or credits
 *                 at all. Their developer forum has several people quoting
 *                 their own limits as "1,000 inference credits (signup), 40
 *                 RPM", which reads like a one-time pool rather than a daily
 *                 one. If that is right, an account is worth a thousand
 *                 requests ONCE and a second account is worth almost nothing —
 *                 which is the opposite of how this file has been treating it.
 *                 UNRESOLVED. Nothing here should be trusted until it is.
 *   Groq          30 a minute, 14,400 a day.
 *   Google        5-30 a minute, 9,000 a day on Flash.
 *   OpenRouter    20 a minute, and 50 a day UNTIL ten dollars has been spent
 *                 on the account at any point, after which it is 1,000 a day
 *                 for good. Their docs also say extra accounts and extra keys
 *                 change nothing, because the limit is on the account holder
 *                 and not the key. One account, ten dollars, done.
 *
 * Measured here rather than read:
 *
 *   NVIDIA answers a request with `usage.prompt_tokens_details.cached_tokens`,
 *   and on the very first call from a brand-new key it reported 32 of 36
 *   prompt tokens served from cache. They cache by prefix, across accounts.
 *   The launcher sends about 31,000 characters of unchanging rules and tool
 *   schemas in front of roughly 2,000 that vary — but the varying part sits in
 *   the MIDDLE, which cuts the usable prefix to about 4,700. Moving what is
 *   specific to a player to the end would hand the rest of it to that cache.
 *
 *   Their free endpoints also return `503 ResourceExhausted: Worker local
 *   total request limit reached (17/16)`. That is their box for that model
 *   being full, not an allowance of ours running out, and it arrives on a
 *   first request as readily as a thousandth. Anything that reads a 503 here
 *   as "we are out of quota" will be wrong.
 *
 *   And the number that matters for planning: forty a minute is what they
 *   publish, not what a key delivers. A brand-new one, fed llama-3.3-70b at a
 *   submitted 35 a minute with about nine calls in flight, got 12 answers and
 *   37 `429 Too Many Requests` in 84 seconds. Nine a minute, near enough —
 *   a quarter of the published figure. Whether the ceiling is the rate or the
 *   number in flight was not separated out.
 *
 *   So twenty-five accounts are worth something like 225 requests a minute
 *   between them, not the 1,000 the published figure implies. Capacity sums
 *   built on `daily` or on 40 RPM are four times too generous.
 *
 * Still not known: whether an account has a finite pool of requests at all.
 * A burn of 49 calls on a fresh key produced no refusal about WHO was asking —
 * only about how fast — and 12 successes says nothing about a pool of 1,000.
 * Settling it needs a couple of hours at nine a minute, and it must not run
 * while anything else is using the pool: it fills the same shared boxes, and
 * a battery run alongside it failed three questions with "every lane refused"
 * that had nothing wrong with them.
 */
const LANES = [
  // Groq: 73 ms from here, and 14,400 a day. The deepest allowance in the pool.
  //
  // Llama is weighted below the others on what it wrote rather than on what it
  // is. Asked twice how to beat Radahn, with both wiki articles in front of it,
  // it produced a paragraph about being patient, gave him a holy-damage second
  // phase he does not have, and closed by saying it could not find enough to
  // give more detail — while Mistral and Nemotron answered the same kind of
  // question with the actual phase thresholds. It is fast and there is a lot of
  // it, so it stays; it goes second.
  { id: "groq/llama-3.3-70b", secret: "GROQ_KEY", provider: "groq", weight: 0.7, daily: 13000,
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
  // Minimax was kept only for capacity when everything else was rate-limited.
  // That reason is gone: the pool went from 27 keys to 267, and it was the
  // worst lane in it — three bad answers (a tool call written out as prose,
  // vanilla co-op advice to a Seamless player, Farum Azula placed in Caelid),
  // and 39-54s against mistral's 1.5s. `enable_thinking:false` does not help;
  // measured four times, it ADDS reasoning output and saves nothing.

  // 1,000 a day, not the 45 this used to say.
  //
  // Their published table: under ten credits bought, ever, it is 50 a day;
  // from ten credits it is 1,000 a day, and it counts purchases over all time
  // rather than the balance, so it stays at 1,000 once the credit is spent.
  // Twelve dollars went on in August 2026. Twenty a minute either way, which
  // makes this a lane for volume and not for a busy evening.
  //
  // Their own refusal said so, which is a pleasant way to have a limit
  // documented: "Rate limit exceeded: free-models-per-day. Add 10 credits to
  // unlock 1000 free model requests per day".
  //
  // One account only. Their docs: "Making additional accounts or API keys will
  // not affect your rate limits, as we govern capacity globally." A second key
  // here would buy nothing at all.
  { id: "openrouter/nemotron", secret: "OPENROUTER_KEY", provider: "openrouter", weight: 0.9, daily: 1000,
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
  //
  // Still true after the prompt was told in as many words not to. Asked where
  // to go next it placed the player in "Всхолодающая Пустошь", which is not a
  // place, spelled Ailing with a Cyrillic А inside a Latin word, and finished
  // with "проверьте карту на маркеры (place_marker)" — handing the reader the
  // internal name of a tool as though it were something they could type.
  //
  // Filtering that out of the stream was considered and dropped: the cleaner
  // releases text only at settled points and a name can arrive split across two
  // of them, so catching it reliably means holding prose back, and mangling
  // every good lane's output to tidy the worst one is the wrong trade. The
  // weight is the mechanism. This is what it is for.
  { id: "cloudflare/qwen3-30b", edge: true, provider: "cloudflare", weight: 0.12, daily: 500,
    assume: 7600, model: "@cf/qwen/qwen3-30b-a3b-fp8" },
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
/**
 * Give up after this many lanes rather than walking the whole list.
 *
 * Five rather than four because the lanes are now spread one per company-and-
 * model, so five attempts are five genuinely different things to try, and a
 * lane that is out of allowance says so in a fifth of a second — the cost of
 * one more is small next to telling somebody nothing could answer them.
 */
const MAX_LANES = 5;
/**
 * How far the streaming path may go when nothing has answered yet.
 *
 * Only reached lane by lane, and only while the player has seen no words at
 * all — see `askPoolStreaming`. A pool in good health never gets past the
 * fifth.
 */
const DEEP_LANES = 9;
/**
 * How long a lane that is already writing may go silent before it is dropped.
 *
 * Before a lane speaks, the clock measures the whole attempt: silence is all
 * there is, and twenty-five seconds of it means nothing is coming. Once it is
 * writing, that same clock is wrong, and measurably so — a model part-way
 * through a long answer had its connection cut at the twenty-five second mark
 * and the player was left reading a sentence that stopped in the middle of a
 * word, with nothing to say it had been cut. Working slowly is not being stuck.
 *
 * So from the first token the clock measures the gap between tokens instead,
 * with an outer bound underneath it in case something streams forever.
 */
const QUIET_FOR_MS = 12_000;
const WHOLE_ANSWER_MS = 90_000;

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
    // `tooBigAt` is the smallest request this lane has ever refused on size.
    // Infinity until one does. See `isTooBig`.
    state = {
      ema: null,
      fails: 0,
      coolUntil: 0,
      used: 0,
      day: today(),
      ok: 0,
      tooBigAt: Infinity,
    };
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

/**
 * Lane health, kept somewhere the next isolate can find it.
 *
 * The map above lives and dies with one isolate, and Cloudflare makes a great
 * many of them, so most requests were arriving at a worker that had never
 * measured anything. `order` then had nothing to sort on but a flat guess, and
 * which lane went first was effectively arbitrary — the same question, asked
 * four times in an afternoon, came back in 7.4s, 18.5s, 42.7s and 7.6s purely
 * according to who happened to be picked. Measuring lanes carefully and then
 * throwing the measurements away every few minutes is not measuring them.
 *
 * The cache is per-datacentre, which is exactly the right shape: how quick a
 * provider is depends on where you are asking from, so figures gathered here
 * are worth having here and nowhere else.
 *
 * Everything about this is best-effort. A miss, a parse failure, a cache that
 * refuses the write — each leaves the pool behaving precisely as it did before
 * any of this existed, because the alternative is a caching bug that stops
 * people getting answers.
 */
const HEALTH_URL = "https://roundtable.invalid/lane-health";
/** Whether this isolate has already looked. Once is enough. */
let healthRead = false;

async function rememberedHealth() {
  if (healthRead) return;
  healthRead = true;
  try {
    const held = await caches.default.match(new Request(HEALTH_URL));
    if (!held) return;
    const saved = await held.json();
    for (const [id, state] of Object.entries(saved)) {
      // Only what is worth carrying. `ok` and `fails` are about one isolate's
      // luck; `ema` is about the provider, and `coolUntil` is an absolute time
      // that is still true whoever reads it.
      const mine = laneState(id);
      if (typeof state.ema === "number" && state.ema > 0) mine.ema = state.ema;
      if (typeof state.used === "number" && state.day === today()) mine.used = state.used;
      // Cooldowns carry over, but only up to an hour of one.
      //
      // A lane whose refusal reads as "out of allowance" is put away for a
      // whole day, and that judgement is made from the wording of a 429 —
      // Google says "you exceeded your current quota" both when the day's
      // allowance is gone and when you were merely too quick a moment ago, and
      // nothing in the message tells the two apart. Inside one isolate a wrong
      // guess cost a few minutes. Written down where every isolate in the
      // datacentre reads it, the same wrong guess would cost a working lane for
      // the rest of the day.
      //
      // So the short cooldowns, which are the accurate ones, survive intact,
      // and a day-long ban comes back as an hour. If the lane really is spent,
      // it says so again an hour later at the price of one fast refusal.
      if (typeof state.coolUntil === "number") {
        mine.coolUntil = Math.min(state.coolUntil, Date.now() + 3600_000);
      }
      // A size refusal is a fact about the lane, and it belongs here with
      // `ema` rather than with the luck.
      //
      // A question gets `MAX_LANES` attempts and no more. Groq's two lanes are
      // the fastest in the pool, so they sort to the very front of every
      // question — and they refuse every real one, 15,866 tokens against an
      // 8,000 limit. `tooBigAt` exists to stop asking them, but it died with
      // the isolate, so a cold one paid the lesson again: measured on a live
      // battery, five refusals of which two were groq saying 413, leaving
      // three real attempts out of five.
      //
      // This needs none of the hedging above it. A 413 states the size
      // outright and there is no second reading of it the way there is of
      // Google's "you exceeded your current quota". Should a limit ever be
      // raised, the 15-minute lifetime of this cache is what notices.
      if (Number.isFinite(state.tooBigAt)) {
        mine.tooBigAt = Math.min(mine.tooBigAt, state.tooBigAt);
      }
    }
  } catch {
    // Nothing was remembered. That is the state everything already handles.
  }
}

function keepHealth(ctx) {
  if (!ctx?.waitUntil || health.size === 0) return;
  try {
    const out = {};
    for (const [id, state] of health) {
      out[id] = { ema: state.ema, coolUntil: state.coolUntil, used: state.used, day: state.day };
      // Only once there is a real figure: `Infinity` does not survive
      // `JSON.stringify`, which turns it into `null`, and a null read back as
      // a size would make every lane look as though it had refused everything.
      if (Number.isFinite(state.tooBigAt)) out[id].tooBigAt = state.tooBigAt;
    }
    ctx.waitUntil(
      caches.default.put(
        new Request(HEALTH_URL),
        new Response(JSON.stringify(out), {
          headers: { "content-type": "application/json", "cache-control": "max-age=900" },
        }),
      ),
    );
  } catch {
    // Same as above: worth trying, never worth failing over.
  }
}

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
    // `_2` upward, until one is missing. The ceiling is high because the
    // limit is how many people are willing to hand over an account, not
    // anything here — and a key that is stored but silently not looked at is
    // the worst possible bug, since nothing fails and capacity simply is not
    // there. It was 40 and 138 keys arrived at once; a ceiling below the count
    // is that exact silent bug, so it sits well above what is loaded and moves
    // up before the keys reach it, never after.
    for (let n = 2; n <= 300; n++) {
      const secret = `${lane.secret}_${n}`;
      if (!env[secret]) break;
      out.push({
        ...lane,
        id: `${lane.id}#${n}`,
        secret,
        provider: `${lane.provider}#${n}`,
        // What this lane is a second copy OF. `provider` deliberately differs
        // per account so a 429 on one never cools the other, and that is right
        // — but it also made every extra account look like a new company to
        // anything spreading requests around. See `family` in `order`.
        family: lane.id,
      });
    }
  }
  return out;
}

/**
 * What this model has cost elsewhere, averaged over the lanes that have run it.
 *
 * `null` when none of them has, which is the only case that deserves a guess.
 */
function typicalFor(env, model) {
  let total = 0;
  let seen = 0;
  for (const lane of withKeys(env)) {
    if (lane.model !== model) continue;
    const { ema } = laneState(lane.id);
    if (ema !== null) {
      total += ema;
      seen += 1;
    }
  }
  return seen === 0 ? null : total / seen;
}

/**
 * The lanes worth trying, best first.
 *
 * A lane that has never been tried is given an optimistic estimate so it gets
 * one chance early; after that it is ranked on what it has actually done. The
 * score divides by weight, so a better model wins ties and a much better model
 * wins even when it is somewhat slower.
 */
function order(env, needsTools = false, depth = MAX_LANES, size = 0, warm = null) {
  const now = Date.now();
  return withKeys(env)
    // A lane that has refused a tool call once is not asked again this
    // isolate's lifetime. Providers differ about which models take tools and
    // the only reliable way to find out is to be told no.
    .filter((lane) => !(needsTools && toolless.has(lane.id)))
    .map((lane) => ({ lane, state: laneState(lane.id) }))
    .filter(({ lane, state }) => state.coolUntil <= now && state.used < lane.daily)
    // And one that has refused a request this big. Not a black mark: it will
    // take a smaller one happily, and the next question usually is smaller.
    // See `isTooBig` for what this was costing while it went unrecorded.
    .filter(({ state }) => size < state.tooBigAt)
    .map((entry) => {
      const { lane, state } = entry;
      // What this lane is expected to cost. Measured once it has run; until
      // then, what the same model costs on the other accounts.
      //
      // A flat optimistic guess put every untried lane at the front, and with
      // ten accounts per model that meant the slowest model kept buying its way
      // back to the top with a fresh account — answers of one second and of
      // twenty-six, in no pattern the player could see. The model is the thing
      // that is slow, not the account.
      // `assume` is for a lane that will never have a sibling to learn from,
      // where the flat guess is simply wrong. Cloudflare's own model is the
      // only one: nothing else runs it, so `typicalFor` is always null, and 400
      // put it near the front of the queue every time an isolate started cold —
      // which is most of the time. It won four races out of eight in one run,
      // and it is the lane that invents place names and mistranslates fire
      // damage as fireworks. Measured, it takes about seven and a half seconds,
      // because it is a reasoning model and spends the budget thinking. Saying
      // so is not a penalty; it is the figure.
      const ms = state.ema ?? typicalFor(env, lane.model) ?? lane.assume ?? 400;
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
    // One lane per company-and-model, and the best of them.
    //
    // Walking down a list that ran groq, groq, nvidia, nvidia meant two of the
    // four attempts were spent asking somewhere that had just said no, and with
    // the retries that follow, every lane ended up cooling down at once. So the
    // attempts are spread.
    //
    // This used to spread on `provider` and quietly stopped working the day a
    // second key was added, because an extra account carries its own provider
    // name — on purpose, so a 429 on one does not cool the other. Three
    // accounts of one model therefore read as three different companies, and a
    // real failure showed exactly that: gpt-oss-120b, #2 and #3 took three of
    // the four attempts, all three came back 429 with "rate limit reached for
    // model openai/gpt-oss-120b", and the pool gave up without ever reaching
    // Mistral or NVIDIA, both of which were answering fine.
    //
    // `family` is the lane an account is a copy of, so the spread is by what is
    // actually being asked — a company and a model — which is also the level a
    // rate limit is usually enforced at.
    .filter(
      (entry, at, all) =>
        all.findIndex((o) => (o.lane.family ?? o.lane.id) === (entry.lane.family ?? entry.lane.id)) ===
        at,
    )
    // The lane that answered the last round of this question goes first, if it
    // survived every filter above.
    //
    // Because a prefix cache belongs to one provider AND one account, and this
    // pool spreads work on purpose — see the tie-break above, which hands the
    // next request to the least-used account so ten accounts are really ten.
    // That spreading is right across QUESTIONS and wrong within one: measured,
    // two consecutive rounds of a single question went to
    // nvidia/llama-3.3-70b and then mistral/medium#2, so each warmed a cache
    // the next round threw away. Where a lane did get a second visit the hit
    // rate was 98% — 13,824 of 14,067 prompt tokens — against nothing at all
    // for the lanes that never got one.
    //
    // Only a nudge to the front, not a pin: the lane still had to pass the
    // cooldown, allowance and too-big filters to be here, and the hedge behind
    // it is untouched, so a warm lane that has gone slow costs one hedge
    // interval and nothing else.
    .sort((a, b) => (a.lane.id === warm ? -1 : 0) - (b.lane.id === warm ? -1 : 0))
    .slice(0, depth);
}

/**
 * A provider that has just refused, and everything on its key with it.
 *
 * A token-per-minute limit is the whole account's, so the sibling lane is
 * already over it too. Cooling them together stops the next round walking
 * straight into the same wall.
 */
/**
 * A lane that has just refused something for being too big.
 *
 * Only this lane, not the provider: size limits are the model's, and the
 * sibling running a different model on the same key is usually fine with it —
 * `gpt-oss-120b` refuses what `llama-3.3-70b` on the same account accepts.
 * The smallest refused size is what gets kept, because a lane that turned down
 * 39,000 characters will also turn down 40,000.
 */
function rememberTooBig(id, size) {
  const state = laneState(id);
  state.tooBigAt = Math.min(state.tooBigAt, size);
}

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

Two of them outrank the rest and are easy to forget. gear_numbers reads the
tables the player's own installation runs on, which is the only correct source
for what a weapon does — the wiki and the item database are both the base game
and are simply wrong under a mod. And search_web exists for what no mirror can
hold: a patch note from last week, a build somebody posted, a mod too new to
have a wiki. When the wikis come back with nothing and the question is about the
world as it is today rather than as it was written down, search the web before
falling back on memory. Answering "I could not find it, but from memory…" with
an untouched web search is a worse answer than the one search would have given.
A search on its own is half the job: it returns a title and one sentence, so
open the promising result with read_page and answer out of the page. A list of
links is not an answer, and neither is a summary of what the links are called.

Two of them touch the map. map_markers reads it, which is free and worth doing
whenever they ask what they have marked — or before pinning, so you can tell
them it is already there instead of pinning it twice. place_marker writes a pin
onto their own map, in their own save. Treat it differently from every other tool. Use it only
when they have asked for a marker, one place per request, and never to decorate
an answer: a route with five stops is not five pins unless they asked for the
route marked. Say afterwards what was pinned and whose map it went on. If it
comes back refused — the game is open, the place is a legacy dungeon, the save
holds more than one character — relay that instead of claiming a marker exists,
because they will go and look.

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

When the search tells you a word of yours appears in no title anywhere, believe it and
take the near spellings it offers seriously. That is a name you got wrong, and the
ranking will happily hand you a different thing with a similar name — asked for
Rellana it returned Rennala, another boss two letters away, and the answer that
followed was about the wrong character.

WHEN THE PLAYER CONTRADICTS YOU
They are looking at the game and you are not. Check before you disagree, and expect to
find that you were both right about different things: told Radahn was in Caelid, the
honest answer is that his arena sits inside it, not that they are mistaken. If they
turn out to be right, say so plainly and move on — no defending the earlier answer.
Only hold your ground when you have just read something that says otherwise, and then
say where it came from.

LANGUAGE
Reply in the same language as the player's last message. Every word of it, including
anything you took out of an English wiki. Russian gets Russian, Ukrainian gets
Ukrainian, Japanese gets Japanese. Never switch to English because the source material
was English.

Names of things are the exception to translating: use the name printed in their own
game, which game_item gives you, and put the English beside it the first time. Do not
translate a wiki's English name yourself — that invents a name they have never seen.
The arena the wiki calls the Wailing Dunes is "Воющие дюны" in a Russian copy, and
translating it produced "Стонущие дюны", which is good Russian and wrong.

The attributes come with their short form — Faith (FTH), Arcane (ARC), Mind (MND) —
and the short form is what to write. It is the same in every language, which is the
point: a Russian copy labels Mind "Интеллект" and Intelligence "Мудрость", so an
invented word for one of them is unreadable. Arcane was once rendered "Тьма", which
is no attribute at all. Write ARC, or the word the game uses if you have been given
it, and never a translation of your own.

ANSWERING
What you looked up outranks anything you remember — the game has been patched many
times and a mod changes numbers outright. Where you could not find it, say plainly
that it is not there, then answer from your own knowledge and mark it as memory.

Answer with what you have. A page you read is rarely everything about a thing and
it does not have to be: if it gave you the phases and not the resistances, give
them the phases. "I could not give more detail because the article did not cover
it" is a sentence about you, and they asked about a boss. Say what you know, in one
short clause say what you could not confirm, and stop.

"How things work" is not a licence to answer from memory, and it was being read as
one. Two questions in a row — what endurance does beyond stamina, and how to
respec — were answered without a single search, and both answers were wrong: the
first handed endurance the poise that comes from armour, the second declared that
this mod has no way to redistribute points at all. A mechanic is exactly the kind
of thing a total conversion rewrites, and exactly the kind of thing a wiki page
states plainly. If the question is about how something works in THIS game, look it
up like anything else.

Memory is for how things work, never for figures. If the player names an item and
the searches came back empty, the honest answer is that it is not in this game —
say that and stop. Do not then supply its damage "from memory": you have just
established you cannot find it, and a number invented in that position is the most
convincing kind of wrong. This has happened: asked about a sword that does not
exist, the answer was that it deals "around 650-700 physical". Nobody was helped.

Never invent a number, an item name or a place. Being vague is fine; being
confidently wrong about a boss costs somebody a run.

Never invent a source either. "Players report", "the community found", "it is
known that", "according to reports" — every one of those is a claim about
somebody else's evidence, and if you did not read it this turn you are making it
up in a form that cannot be checked. Asked why a patch had changed a boss, the
search found no such change and the answer said so, then added that players had
noticed it anyway and that it had simply gone unannounced. That is worse than
agreeing with the false premise outright, because it explains away the very
evidence that contradicts it.

When the question takes something for granted that you cannot find, say you
cannot find it and stop. Do not supply a reason it might be true anyway. The
honest shape is "there is nothing about that in the notes, and nothing in the
searches — where did you see it?"

EVERY NAME YOU WRITE CAME FROM A TOOL
Before you name an item, a spell, a talisman, a place or an NPC, it must have come
back from a tool in this conversation. Not from memory, however sure you feel. If you
did not look it up, describe it instead — "a talisman that raises bleed" costs the
reader nothing, and a made-up name sends them hunting for something that does not
exist.

This is the failure that keeps happening, and length is what causes it: asked what to
level, the answer ran long and filled the space with "Огненная грешная кожа", "Коготь
Морина" and "Зелёная черепашка" — none of which exist, all of which read as certain.
A long answer must be long because you looked several things up, never because you
remembered more.

So: search first, then write. If you have looked nothing up, you have a short answer,
and a short true answer is the better one. If a question needs five item names, that
is five things to look up, not five things to recall.

Do not describe the looking. "According to the wiki", "based on the passages", "I
searched for" — none of that is the answer. Say the thing. The one exception is when
sources disagree or when you are going from memory, which the player does need told.

Never write the name of a tool. gear_numbers, player_status, game_item, search_wiki
and the rest are machinery the player has never heard of and cannot use — telling
them "the weights can be added up through gear_numbers" reads like an instruction and
is not one. Say what you can do for them instead: "I can add them up if you want."
Same for the launcher's own parts. Talk about the game and about what you will do,
never about how you are wired.

Answer the question that was asked, then give them the next thing they will need —
the caveat that matters, what this sets up, what usually goes wrong here. Not every
location of every Golden Seed when they asked how to get more flasks, but not a bare
line either.

Length follows what you looked up, never the other way round. Three articles read is
an answer with something in it; nothing read is two honest sentences. Do not reach a
length by adding things you did not check. No preamble and no restating the question.

TALKING TO THEM
Like somebody who plays the game sitting next to them, not a reference book. They are
mid-run with a controller in their hands. Match how they write: short and blunt gets
short and blunt back, swearing included if that is how they talk.

Everything you are handed talks ABOUT the player — "their stats", "on their screen",
"they are holding" — because it is notes written to you. Your answer talks TO them.
Turn every one of those round: "on their screen" is "on your screen". Copied across
as it stands it produces a report about a third party delivered to their face, which
is what happened — "Урон на их экране: 45" to the person whose screen it is.

Ask when it would change the answer. What a boss is doing to them, what they are
carrying, how far they have got — one question, at the end, only when the answer
genuinely turns on it. Never ask instead of answering: give the best answer you can
first, then ask what would sharpen it.

Above all, never ask for something a tool would tell you. "Which piece did you mean —
or I can check what you are wearing" is not a question, it is the tool you were about
to call, handed back to the player as homework. Asked its frame cap and its armour
weight in one breath, the answer gave the cap and asked which piece of armour was
meant — the same question on its own had been answered by looking, correctly, a minute
earlier. Two things asked at once are two things to look up, not grounds to answer one.

Have an opinion. "Both work, but at your level the seal is the easier fight" is worth
more than two options and no view. Say when something is a bad idea, and say when they
have picked something good.

You know their character — level, stats, what they are holding, where they are
standing. Use it. "At 22 Faith you are two levels off that incantation" is the answer;
"it requires 24 Faith" is a page they could have read themselves.

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

/**
 * A refusal about the SIZE of the request rather than about anything wrong.
 *
 * `413 Request too large for model openai/gpt-oss-120b` is what the highest
 * weighted lane in this pool was answering with, every time, for weeks. It was
 * being counted as a failure, which cooled the lane for a few seconds and then
 * let it fail again in exactly the same way, and the question fell through to
 * whatever was slowest — measured, 18.9 seconds against gpt-oss's usual sub-
 * second. From the launcher it looked like an exhausted pool. It never was: a
 * 354-byte question answered through the same lane in 825 milliseconds while a
 * 39,000-character one was being turned away.
 *
 * It is not a fault and it is not transient. The same body will be refused
 * forever, and a smaller one will not be — so what the pool needs to remember
 * is the size, not a black mark.
 */
function isTooBig(status, body) {
  return status === 413 || /request too large|too many tokens|context length/i.test(body);
}

/** Room to answer properly, and room to think before calling a tool. */
const MAX_TOKENS = 1600;

/**
 * One turn, which may come back as an answer or as a tool the model wants run.
 *
 * The tools themselves live in the launcher, because the wikis do — this only
 * carries the request out and the model's decision back.
 */
/**
 * The opt-in Mistral needs before it will cache anything.
 *
 * Measured first, then read. Six rounds of battery 49 reported 0, 0, 0, 0, 0 and
 * 32 cached prompt tokens out of fourteen to fifteen thousand — effectively
 * nothing, on lanes that were answering perfectly well. Mistral's documentation
 * says why: caching is OPT-IN. "To enable caching, you opt in by passing a
 * stable prompt_cache_key and keeping the shared prefix identical across
 * calls." We never passed one, so there was never a cache to hit.
 *
 * That is worth having. About 45,000 characters at the head of every request
 * are the same every time — the rules block and the tool schemas — and Mistral
 * bills a cached token at a tenth of the usual rate. Groq goes further and does
 * not count cached tokens against the tokens-per-minute ceiling at all, which
 * is the ceiling that has been refusing questions outright with 413.
 *
 * The key is derived from the SYSTEM turn, which is that shared head, so two
 * questions from the same installation get the same key and the second reuses
 * the first. Cheap on purpose: a hash over the string rather than anything
 * clever, because this worker has already been killed once for spending its
 * whole CPU budget, and a hash of 27,000 characters is microseconds only if it
 * stays this dull.
 *
 * Only for lanes known to take the field. An unknown parameter is a 400 from
 * some providers, and a lane refusing outright is worse than a lane not
 * caching.
 */
function cacheKeyFor(lane, messages) {
  if (!/mistral/i.test(lane.url ?? "")) return {};
  const head = messages.find((turn) => turn.role === "system")?.content ?? "";
  if (head.length < 512) return {};
  // FNV-1a, 32 bits. Not a security hash and does not need to be.
  let hash = 0x811c9dc5;
  for (let i = 0; i < head.length; i++) {
    hash ^= head.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return { prompt_cache_key: `roundtable-${hash.toString(36)}` };
}

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
      ...cacheKeyFor(lane, messages),
      ...(tools ? { tools, tool_choice: "auto" } : {}),
    }),
    signal,
  });

  if (!res.ok) {
    const body = (await res.text()).slice(0, 200);
    throw Object.assign(new Error(`${res.status} ${body}`), {
      lane: lane.id,
      exhausted: isExhausted(res.status, body),
      tooBig: isTooBig(res.status, body),
    });
  }

  const said = await res.json();
  const message = said.choices?.[0]?.message ?? {};
  const calls = normaliseCalls(message.tool_calls);
  // The answer, and only the working when there is no answer — a reasoning
  // model can leave `content` empty and put everything in the other field,
  // which looks like success and reads as silence.
  const text = (message.content || message.reasoning_content || "").trim();
  if (!text && calls.length === 0) {
    throw Object.assign(new Error("empty answer"), { lane: lane.id });
  }
  // How much of the prompt was a cache hit, and how much was sent in total.
  //
  // Worth carrying all the way back because of what Groq's own documentation
  // says: cached tokens do NOT count towards the tokens-per-minute limit, and
  // that limit is what has been killing questions — four of eight in one
  // battery, refused with 413 "Request too large" by the fastest lanes. The
  // stable head of every request here is about 45,000 characters, so if it is
  // hitting the cache it stops counting, and nothing has to be cut.
  //
  // Nothing reported this, so whether it hits was pure guesswork. Now it is a
  // number. Providers put it in different places; both spellings seen in the
  // wild are read, and a lane that reports neither comes back as null rather
  // than as zero, because "did not say" and "nothing cached" are not the same.
  const usage = said.usage ?? {};
  const cached =
    usage.prompt_tokens_details?.cached_tokens ??
    usage.cached_tokens ??
    null;
  return {
    text,
    calls,
    ms: Date.now() - started,
    lane: lane.id,
    prompt: usage.prompt_tokens ?? null,
    cached,
  };
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
      tooBig: isTooBig(res.status, body),
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
/**
 * The two patterns `settled` matches at a position, both sticky.
 *
 * `y` anchors the match at `lastIndex` instead of searching, which is what
 * makes it possible to ask "does a marker start HERE" without cutting the rest
 * of the string off first. Module-level so they are compiled once; nothing
 * awaits between setting `lastIndex` and reading the result, so sharing them
 * across requests in an isolate is safe.
 */
const LINE_MARKER = /[ \t]*(#{1,6}|>|[-*+]|\||\d+\.|[-_*]{3,})/y;
const RUN = /(\*{1,3}|`{1,3})/y;

// Named so `bench/cleaner.mjs` can time the real thing rather than a copy of
// it. A copy would have gone on being linear while this one was not.
export { clean };

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

  /**
   * Does `held` read as `trap.open` starting at `at`, ignoring case?
   *
   * Written out rather than `held.slice(at).toLowerCase().startsWith(...)`
   * because that copies everything from `at` to the end of the buffer, and it
   * used to be asked once per character. See the note on `settled`.
   */
  const opensAt = (open, at) =>
    at + open.length <= held.length &&
    held.slice(at, at + open.length).toLowerCase() === open.toLowerCase();

  /** Could it still become one, if more characters arrived? */
  const mayOpenAt = (open, at) => {
    const left = held.length - at;
    // Only when what is left is SHORTER than the opener — otherwise the
    // question is already answered by `opensAt`. That bound is what keeps this
    // from copying the rest of the buffer.
    return left > 0 && left < open.length &&
      open.toLowerCase().startsWith(held.slice(at).toLowerCase());
  };

  /**
   * How far into `held` everything is settled.
   *
   * WHY THIS IS WRITTEN WITH INDICES AND NOT `slice`. This walks the buffer one
   * character at a time, and it used to ask its questions by cutting the rest
   * of the buffer off and matching against that — `/^(\*{1,3}|`{1,3})/.exec(
   * held.slice(at))`, once per character. Each of those copies everything from
   * `at` to the end, so the work was quadratic in the length of `held`.
   *
   * BE CLEAR ABOUT WHAT THIS DID AND DID NOT FIX. It was changed while hunting
   * the thing that takes this worker over Cloudflare's CPU limit — the failures
   * log as `outcome: exceededCpu`, `cpuTime: 32500ms`, against 8-23ms for a
   * request that answers. The copying was real and is gone. It was NOT the
   * cause: `bench/cleaner.mjs` times the version before this change and the
   * version after, at five sizes up to eighty thousand characters, on
   * marked-up text and on unbroken prose, and BOTH are linear and both finish
   * in single-digit milliseconds. The buffer is released in small pieces long
   * before the copying could matter.
   *
   * So this is a tidier walk and nothing more, and whatever spends thirty-two
   * seconds of CPU is still out there. Do not read this comment as the answer.
   *
   * The sticky regexes below match AT a position without copying anything, and
   * `opensAt` copies at most the length of one opener. Same behaviour, linear.
   */
  function settled() {
    let at = 0;
    let lineStart = atLineStart;

    while (at < held.length) {
      const ch = held[at];

      // A line whose first characters could make it a heading, a quote, a
      // bullet or a table row is not settled until the line ends: the rules
      // for those need the whole line, and `#` on its own is just a character.
      if (lineStart) {
        LINE_MARKER.lastIndex = at;
        const marker = LINE_MARKER.exec(held);
        if (marker) {
          const ends = held.indexOf("\n", at);
          if (ends === -1) return at;
          at = ends + 1;
          continue;
        }
      }
      lineStart = ch === "\n";

      if (ch === "<") {
        // A whole opener: handled by the caller, which enters swallow mode.
        if (traps.some((t) => opensAt(t.open, at))) return at;
        // Might still become one, given more characters.
        if (traps.some((t) => mayOpenAt(t.open, at))) return at;
        at += 1;
        continue;
      }

      // An emphasis or code run, which means nothing until it closes.
      RUN.lastIndex = at;
      const run = RUN.exec(held);
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
  return (
    text
      .replace(/\*\*\*(.+?)\*\*\*/gs, "$1")
      .replace(/\*\*(.+?)\*\*/gs, "$1")
      .replace(/(^|[^*])\*([^*\n]+?)\*(?!\*)/g, "$1$2")
      .replace(/`{1,3}([^`]+?)`{1,3}/gs, "$1")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      // Maths, which nothing here asked for. Some models reach for it as
      // punctuation and an answer about a boss arrived carrying a literal
      // "$\rightarrow$" between two place names.
      .replace(/\$\\(?:rightarrow|to|Rightarrow)\$/g, "→")
      .replace(/\$\\(?:times|cdot)\$/g, "×")
      .replace(/\\\((.+?)\\\)/gs, "$1")
      .replace(/\$([^$\n]{1,40})\$/g, (whole, inner) =>
        /\\/.test(inner) ? inner.replace(/\\[a-zA-Z]+\s?/g, "") : whole,
      )
  );
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
  // Deeper than the usual cut-off, and only reached while nobody has said
  // anything at all. `MAX_LANES` is there to stop a question quietly costing
  // twenty requests; it is not worth a player sitting in front of a blank
  // overlay. Measured on the same question over an afternoon, the answer came
  // back in seven seconds and in forty-three, entirely according to which lane
  // won — and at forty-three every one of the five had been started and none
  // had spoken, with eighty more sitting idle behind the cut-off.
  //
  // The extra ones are started one at a time on the same pacing as the rest, so
  // a pool that is answering normally never reaches them.
  // With the size, like the other one. Without it this path kept sending the
  // final round — the one that writes the actual prose, and the slowest of the
  // lot — to lanes already known to refuse a request that big. Measured: three
  // tool rounds at three seconds each and then twenty-two for the answer.
  const size = JSON.stringify(messages).length;
  const lanes = order(env, false, DEEP_LANES, size);
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
    const spread = Math.round(HEDGE_AFTER_MS * Math.max(1, leader / Math.max(nextUp, 0.05)));
    // Past the usual cut-off, several seconds have gone by with nobody saying
    // anything, so the careful spacing has already been tried and has not
    // worked. Waiting another two seconds per lane to be polite about it just
    // adds to what the player is sitting through.
    return next >= MAX_LANES ? Math.min(spread, 600) : spread;
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
          tallyFailure(done.id, done.error, env, size);
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
      tallyFailure(settled.id, settled.error, env, size);
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

  // The winner is writing, so its deadline changes shape: see QUIET_FOR_MS.
  const win = controllers.get(winner);
  let quiet = null;
  let cut = false;
  const breathe = () => {
    clearTimeout(quiet);
    quiet = setTimeout(() => {
      cut = true;
      win?.controller.abort();
    }, QUIET_FOR_MS);
  };
  if (win) {
    clearTimeout(win.timer);
    win.timer = setTimeout(() => {
      cut = true;
      win.controller.abort();
    }, WHOLE_ANSWER_MS);
    breathe();
  }

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
      breathe();
      const shown = push(step.value);
      if (shown) yield { delta: shown };
    }
  } catch {
    // A stream that dies partway still leaves the player with what arrived.
    cut = true;
  }
  clearTimeout(quiet);
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
  // `cut` travels with the answer so the player can be told it stops early.
  // Silently handing over half a sentence is the part that was wrong.
  yield { done: true, lane: winner, ms, cut };
}

function tallyFailure(id, error, env, size = 0) {
  // A size refusal is not a fault and cooling the lane for it is wrong twice
  // over: it will take the next question, which is usually smaller, and it will
  // refuse this one again in exactly the same way when the cooldown ends. The
  // other path learned this; this one was still punishing the lane.
  if (error?.tooBig) {
    rememberTooBig(id, size);
    return;
  }
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
/**
 * Why a lane did not answer, in words, including when it threw nothing useful.
 *
 * An abort arrives as a DOMException whose message is "The operation was
 * aborted", which says nothing about whose deadline it was. Naming it means the
 * `tried` list distinguishes a lane that refused from one that simply never
 * came back — and those want different fixes.
 */
function why(error) {
  if (error?.name === "AbortError" || /aborted/i.test(error?.message ?? "")) {
    return "no answer before the deadline";
  }
  const said = error?.message ?? String(error);
  // A refusal on size gets more room, because the numbers are at the END of it
  // and 120 characters stopped just short of them every time. Groq's reads
  // "Request too large for model `X` in organization `org_...` ... Limit 10000,
  // Requested 15088" — and that tail is the whole diagnosis: it says the 413 is
  // a TOKENS-PER-MINUTE ceiling being crossed, not a fixed cap on one request.
  // Which is a different problem with a different fix, and the truncation is
  // why it stayed a guess. Confirmed against Groq's own docs before widening.
  if (/too large|too many tokens|context length|rate limit/i.test(said)) {
    return said.slice(0, 300);
  }
  return said.slice(0, 120);
}

async function askPool(env, messages, deadlineMs = 25_000, tools = null, warm = null) {
  // How big this one is, so lanes that have already refused something this
  // size are not asked again. See `isTooBig`.
  const size = JSON.stringify(messages).length + (tools ? JSON.stringify(tools).length : 0);
  const lanes = order(env, Boolean(tools), MAX_LANES, size, warm);
  if (lanes.length === 0) {
    // Told apart, because they want opposite fixes and the wrong one was being
    // reported: a pool with allowance left that will not take a request this
    // big is a request to make smaller, not a pool to top up.
    const anyAtAll = order(env, Boolean(tools), MAX_LANES, 0).length;
    return anyAtAll > 0
      ? { error: `no lane will take a request of ${size} characters`, tried: [] }
      : { error: "no lane has any allowance left", tried: [] };
  }

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
        // `id` on BOTH sides, and it is the reason this worker stopped being
        // killed for burning CPU. What is settled has to be removed from
        // `running`, and that used to be done with `settled.error.lane` —
        // a property `callLane` puts on the errors it throws ITSELF. The ones
        // it does not throw have no such property: when the deadline below
        // fires, `fetch` rejects with a bare AbortError, and the same goes for
        // a dropped connection or a malformed response body.
        //
        // Then `running.delete(undefined)` removed nothing, the loop went round
        // and raced a promise that had already settled, got the same value back
        // instantly, and did it again — a tight loop at full tilt until
        // Cloudflare killed the request. Its logs read `outcome: exceededCpu`,
        // `cpuTime: 32500ms`, where a request that answers spends 8 to 23ms,
        // and the wall times were 63 to 75 seconds: twenty-five waiting for the
        // abort, the rest spinning. To the launcher that is a 503, and to the
        // player it was the assistant being broken for anything that took more
        // than one lane to answer — ten questions in twelve, on the last count.
        //
        // The id is known HERE, before anything can go wrong, so it is attached
        // here rather than recovered later.
        .then((out) => ({ out, id: lane.id }))
        .catch((error) => ({ error, id: lane.id }))
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
        tried.push({ lane: first.id, why: why(first.error) });
        if (first.error.tooBig) {
          rememberTooBig(first.id, size);
        } else {
          coolProvider(first.id, first.error.exhausted, env);
        }
        running.delete(first.id);
      }
      continue;
    }

    if (settled.out) {
      succeeded(settled.out.lane, settled.out.ms);
      // Whoever else is still going is no longer needed.
      for (const [id, c] of controllers) if (id !== settled.out.lane) c.abort();
      return { ...settled.out, tried };
    }

    const { exhausted, tooBig } = settled.error;
    const lane = settled.id;
    tried.push({ lane, why: why(settled.error) });
    if (tooBig) {
      // Also not a broken lane. It will take the next question, which will
      // almost certainly be smaller — cooling it would only mean the one after
      // that goes somewhere slower for no reason.
      rememberTooBig(lane, size);
    } else if (tools && /tool|function.call/i.test(settled.error.message ?? "")) {
      // Not a broken lane — one that does not do tool calling. It is still
      // wanted for the answer, so it is remembered rather than cooled down.
      toolless.add(lane);
    } else {
      coolProvider(lane, exhausted, env);
    }
    const before = running.size;
    running.delete(lane);
    if (running.size === before) {
      // Nothing came out of the map, so this promise is still in it and the
      // next turn would race it again, settle instantly, and do the same for
      // ever. That is precisely the loop that spent this worker's whole CPU
      // budget, and ids are now attached at the source so it cannot happen —
      // but a loop whose exit depends on a value arriving in the right shape
      // should be able to give up. Ending the request costs one question;
      // spinning costs every question in flight on the isolate.
      return { error: "a lane settled without saying which lane it was", tried };
    }
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
  async fetch(request, env, ctx) {
    if (request.method === "OPTIONS") return new Response(null, { headers: CORS });
    const url = new URL(request.url);
    // What earlier isolates in this datacentre learned about the lanes. Free
    // when it is not there; see `rememberedHealth`.
    await rememberedHealth();

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
        //
        // `version` comes from Cloudflare and cannot be forgotten. `build` is
        // the hand-written one and stays only as a label for the intent of a
        // release — never trust it to tell you what is deployed. It went two
        // deploys out of date on the day the CPU bug was fixed, which is the
        // whole reason the binding is now bound.
        build: BUILD,
        version: env.CF_VERSION_METADATA?.id ?? "unknown",
        deployed: env.CF_VERSION_METADATA?.timestamp ?? null,
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
    // Which lane answered the round before this one, if the caller says.
    //
    // The worker cannot know it: every round of a question is a separate
    // request and nothing is kept between them. But the launcher does know, so
    // it tells us, and `order` puts that lane first. See the note there for
    // what it buys — 98% of a fourteen-thousand-token prompt served from cache
    // instead of nothing, because a prefix cache belongs to one provider and
    // one account and this pool spreads work across eighty-eight of them.
    //
    // Bounded and sanitised: it is only ever compared against lane ids we
    // already have, so a nonsense value simply matches nothing.
    const warmLane = typeof body.prefer === "string" ? body.prefer.slice(0, 80) : null;
    const edition = body.edition ? String(body.edition).slice(0, 60) : null;
    const tools = Array.isArray(body.tools) && body.tools.length > 0 ? body.tools : null;
    const messages = [{ role: "system", content: system(edition) }, ...body.messages.slice(-24)];

    if (!body.stream) {
      // Timed from here, not from whichever lane happened to win.
      //
      // `askPool` reports the winner's own duration, which is the right figure
      // for ranking lanes and the wrong one to show a player: when the first
      // lane spends three seconds failing and the second answers in one, the
      // honest number is four. The launcher prints this under the answer, so it
      // was quietly claiming to be faster than the person watching it.
      // Streaming has always measured the whole thing; this now matches.
      const began = Date.now();
      const result = await askPool(env, messages, 25_000, tools, warmLane);
      keepHealth(ctx);
      if (result.error) return json(result, 503);
      // Tidied the same way the streamed path is. It did not used to be, on the
      // reasoning that this path only ever carried tool calls — and then the
      // launcher started using the answer that comes back with them, rather
      // than asking a second time for something it already had, and every
      // asterisk the models are told not to write arrived in the overlay as an
      // asterisk. One cleaner, both ways out.
      const tidy = clean();
      const content = result.text ? tidy.take(result.text) + tidy.rest() : result.text;
      return json({
        content,
        toolCalls: result.calls ?? [],
        lane: result.lane,
        ms: Date.now() - began,
        tried: result.tried,
        // See `callLane`. Cached tokens do not count against the per-minute
        // ceiling that has been refusing questions, so whether the 45,000
        // stable characters at the head of every request are hitting the cache
        // is the difference between the problem and no problem. Reported so it
        // can be measured instead of assumed.
        prompt: result.prompt ?? null,
        cached: result.cached ?? null,
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
        // Written here rather than beside the `return` below, because the
        // response goes back the moment the stream is handed over and the
        // timings this is worth keeping are not measured until it has run.
        keepHealth(ctx);
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
