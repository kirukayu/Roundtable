/**
 * The answering service.
 *
 * Roundtable mirrors both wikis onto the player's machine, so the searching
 * happens there and costs nothing. What arrives here is a question and the few
 * passages that matched it; all this does is put them to a model and hand back
 * prose. Small requests are the only reason a set of free tiers stretches
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
const BUILD = "2026-08-07.6";

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
  { id: "groq/llama-3.3-70b", secret: "GROQ_KEY", weight: 1.0, daily: 13000,
    url: "https://api.groq.com/openai/v1/chat/completions", model: "llama-3.3-70b-versatile" },
  { id: "groq/gpt-oss-120b", secret: "GROQ_KEY", weight: 1.0, daily: 1000,
    url: "https://api.groq.com/openai/v1/chat/completions", model: "openai/gpt-oss-120b" },

  // Gemini. Google refuses this call from some countries; from here it does
  // not, which is half the reason this service exists.
  { id: "gemini/flash", secret: "GEMINI_KEY", weight: 1.0, daily: 1400,
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
    model: "gemini-flash-latest" },

  { id: "mistral/medium", secret: "MISTRAL_KEY", weight: 0.95, daily: 2000,
    url: "https://api.mistral.ai/v1/chat/completions", model: "mistral-medium-latest" },

  // NVIDIA lists a hundred models and serves a handful. These three answered;
  // everything larger timed out and much of the catalogue 404s.
  { id: "nvidia/llama-3.1-8b", secret: "NVIDIA_KEY", weight: 0.7, daily: 1500,
    url: "https://integrate.api.nvidia.com/v1/chat/completions", model: "meta/llama-3.1-8b-instruct" },
  { id: "nvidia/minimax-m3", secret: "NVIDIA_KEY", weight: 0.9, daily: 1000,
    url: "https://integrate.api.nvidia.com/v1/chat/completions", model: "minimaxai/minimax-m3" },
  { id: "nvidia/deepseek-v4", secret: "NVIDIA_KEY", weight: 0.95, daily: 1000,
    url: "https://integrate.api.nvidia.com/v1/chat/completions", model: "deepseek-ai/deepseek-v4-flash" },

  { id: "openrouter/nemotron", secret: "OPENROUTER_KEY", weight: 0.9, daily: 45,
    url: "https://openrouter.ai/api/v1/chat/completions",
    model: "nvidia/nemotron-3-super-120b-a12b:free" },

  // Cloudflare's own, reached through the binding. Slowest, but it is the one
  // lane that cannot be rate-limited away by somebody else's outage.
  { id: "cloudflare/qwen3-30b", edge: true, weight: 0.85, daily: 500,
    model: "@cf/qwen/qwen3-30b-a3b-fp8" },
];

/** How long a lane sits out after failing, doubling each time. */
const COOLDOWN_MS = 20_000;
const COOLDOWN_MAX_MS = 10 * 60_000;
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
 * The lanes worth trying, best first.
 *
 * A lane that has never been tried is given an optimistic estimate so it gets
 * one chance early; after that it is ranked on what it has actually done. The
 * score divides by weight, so a better model wins ties and a much better model
 * wins even when it is somewhat slower.
 */
function order(env) {
  const now = Date.now();
  return LANES.filter((lane) => (lane.edge ? env.AI : env[lane.secret]))
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
    .sort((a, b) => a.score - b.score)
    .slice(0, MAX_LANES);
}

const SYSTEM = `You answer questions about ELDEN RING and its mods for a player who is
mid-game and wants an answer now.

Use only the wiki passages given to you. If they do not cover it, say so plainly
rather than guessing — a confident wrong answer about a boss or a build costs
somebody a run.

Answer in the language the question was asked in. Be brief: two or three
sentences unless more is asked for. No preamble, no restating the question.`;

/**
 * Turning a question into the words the wiki uses.
 *
 * Kept deliberately narrow. The model is not being asked to answer anything
 * here, only to name things — and naming things is what a model is reliably
 * good at across languages, where a lookup table is not.
 */
const PLANNER = `You turn a player's question into search terms for the ELDEN RING wiki,
which is written in English.

Reply with ONE line: comma-separated English terms, nothing else. No numbering,
no explanation, no quotes.

Give the proper names as the wiki spells them, plus the words its article
headings use. If the question is in another language, translate it. If it names
a thing, that name comes first.

"The Convergence" is a total conversion mod for the game, not a game mechanic.
When somebody names it they are saying which version they are playing, so drop
it and answer for the thing they actually asked about.

Examples:
Как убить Малению? -> Malenia Blade of Miquella, boss, strategy, Waterfowl Dance, Scarlet Rot
Как качаться в Convergence? -> Attributes, Runes, level up, Site of Grace, stats
what's the best bleed weapon -> Rivers of Blood, Bloodhound's Fang, blood loss, Arcane, weapons
где найти коня -> Torrent, Spectral Steed Whistle, Melina, mount`;

function plan(question) {
  return [
    { role: "system", content: PLANNER },
    { role: "user", content: `${question} ->` },
  ];
}

function prompt(question, passages) {
  // The first passage is the best-matching article and gets far more room than
  // the rest. Cutting every one to the same couple of thousand characters kept
  // slicing the section that held the answer in half, and the model then said
  // the passages did not cover it — which was true, and avoidable. Every lane
  // here takes tens of thousands of tokens.
  const context = (passages ?? [])
    .slice(0, 4)
    .map((p, i) => `[${i + 1}] ${p.title}\n${String(p.text).slice(0, i === 0 ? 8000 : 2000)}`)
    .join("\n\n");
  return [
    { role: "system", content: SYSTEM },
    {
      role: "user",
      content: context
        ? `Wiki passages:\n\n${context}\n\nQuestion: ${question}`
        : `Question: ${question}\n\n(No wiki passages matched. Say you could not find it.)`,
    },
  ];
}

/** True when the refusal means "your allowance is gone", not "try again". */
function isExhausted(status, body) {
  if (status === 402) return true;
  if (status !== 429) return false;
  return /quota|balance|insufficient|credit|exceeded your/i.test(body);
}

async function callLane(lane, env, messages, signal) {
  const started = Date.now();

  if (lane.edge) {
    const out = await env.AI.run(lane.model, { messages, max_tokens: 500, temperature: 0.3 });
    const text = (out.response ?? "").trim();
    if (!text) throw Object.assign(new Error("empty answer"), { lane: lane.id });
    return { text, ms: Date.now() - started, lane: lane.id };
  }

  const res = await fetch(lane.url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${env[lane.secret]}` },
    body: JSON.stringify({ model: lane.model, messages, max_tokens: 500, temperature: 0.3 }),
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
  // A reasoning model can leave `content` empty and put the answer in a field
  // of its own, which looks like success and reads as silence.
  const text = (message.content || message.reasoning_content || "").trim();
  if (!text) throw Object.assign(new Error("empty answer"), { lane: lane.id });
  return { text, ms: Date.now() - started, lane: lane.id };
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
async function askPool(env, messages, deadlineMs = 25_000) {
  const lanes = order(env);
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
      callLane(lane, env, messages, controller.signal)
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
        failed(first.error.lane, first.error.exhausted);
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
    failed(lane, exhausted);
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
async function allowed(env, who) {
  // Both windows are checked, so a burst is caught by the first and a long
  // afternoon of steady requests by the second.
  const burst = await env.PER_MINUTE?.limit({ key: who });
  if (burst && !burst.success) return { ok: false, retryAfter: 60, why: "too many at once" };

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
      const lanes = LANES.filter((lane) => (lane.edge ? env.AI : env[lane.secret])).map((lane) => {
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
        LANES.map(async (lane) => {
          if (lane.edge ? !env.AI : !env[lane.secret]) return { lane: lane.id, ok: false, why: "no key" };
          try {
            const r = await callLane(lane, env, messages, AbortSignal.timeout(25_000));
            succeeded(lane.id, r.ms);
            return { lane: lane.id, ok: true, ms: r.ms, said: r.text.slice(0, 40) };
          } catch (error) {
            return { lane: lane.id, ok: false, why: error.message?.slice(0, 130) };
          }
        }),
      );
      return json(out.sort((a, b) => (a.ms ?? 1e9) - (b.ms ?? 1e9)));
    }

    // What to search the wiki for, worked out by a model rather than by a table.
    //
    // The launcher does its own retrieval, and to do that it needs English
    // search terms — both wikis are written in English and a great many players
    // are not. That was a hand-written glossary of Russian words, which had to
    // grow an entry every time something missed and never generalised: it knew
    // "убить" but not "как задавить", and nothing at all in Polish.
    //
    // A model does the whole job properly for the price of one fast call. It
    // sees the question, and returns the words the wiki would use.
    if (url.pathname === "/terms" && request.method === "POST") {
      const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
      const gate = await allowed(env, ip);
      if (!gate.ok) return json({ terms: [] });

      let body;
      try {
        body = await request.json();
      } catch {
        return json({ error: "expected JSON" }, 400);
      }
      const question = String(body.question ?? "").trim().slice(0, 600);
      if (!question) return json({ terms: [] });

      const result = await askPool(env, plan(question), 9000);
      if (result.error) return json({ terms: [], why: result.error });

      // A model told to answer with a list will sometimes explain itself first.
      // Take the last line that looks like a list and ignore the rest.
      const line = result.text
        .split("\n")
        .map((l) => l.replace(/^[^A-Za-z]*/, "").trim())
        .filter((l) => l.length > 0 && !l.endsWith(":"))
        .pop() ?? "";

      const terms = line
        .split(",")
        .map((t) => t.replace(/[^\w' -]/g, "").trim())
        .filter((t) => t.length > 1 && t.length < 40)
        .slice(0, 8);

      return json({ terms, lane: result.lane, ms: result.ms });
    }

    if (url.pathname !== "/ask" || request.method !== "POST") {
      return json({ error: "POST /ask" }, 404);
    }

    const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
    const gate = await allowed(env, ip);
    if (!gate.ok) {
      return new Response(JSON.stringify({ error: gate.why, retryAfter: gate.retryAfter }), {
        status: 429,
        headers: { "content-type": "application/json", "retry-after": String(gate.retryAfter), ...CORS },
      });
    }

    // A body far larger than a question and four passages is either a mistake
    // or somebody using this as a general-purpose model. Neither is wanted.
    const length = Number(request.headers.get("content-length") ?? 0);
    if (length > 60_000) return json({ error: "too much" }, 413);

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "expected JSON" }, 400);
    }

    const question = String(body.question ?? "").trim().slice(0, 600);
    if (!question) return json({ error: "no question" }, 400);

    const result = await askPool(env, prompt(question, body.passages));
    if (result.error) return json(result, 503);
    return json({ answer: result.text, lane: result.lane, ms: result.ms, tried: result.tried });
  },
};
