/**
 * The answering service.
 *
 * Roundtable mirrors both wikis onto the player's machine, so the searching
 * happens there and costs nothing. What arrives here is a question and the few
 * passages that matched it; all this does is put them to a model and stream back
 * prose. That keeps every request small, which is the only reason a free tier
 * stretches across a whole userbase.
 *
 * No key ever ships in the launcher. They live here as secrets, and the launcher
 * only knows this URL.
 *
 * Providers are tried in order and the first one that answers wins. They are
 * ordered by what a player waiting mid-game actually feels: Groq first because
 * it returns in half a second and has by far the largest daily allowance, then
 * the rest as cover for when it is rate-limited or down.
 */

/** Everything here speaks the OpenAI chat shape, so one call fits all of them. */
const PROVIDERS = [
  {
    id: "groq",
    secret: "GROQ_KEY",
    url: "https://api.groq.com/openai/v1/chat/completions",
    model: "llama-3.3-70b-versatile",
    // 14,400 a day and answers in about half a second.
    weight: 1,
  },
  {
    id: "gemini",
    secret: "GEMINI_KEY",
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
    model: "gemini-flash-latest",
    // Blocked from some countries when called directly. From here the call
    // leaves a Cloudflare datacentre instead, which is the point.
    weight: 2,
  },
  {
    id: "mistral",
    secret: "MISTRAL_KEY",
    url: "https://api.mistral.ai/v1/chat/completions",
    model: "mistral-medium-latest",
    weight: 3,
  },
  {
    id: "openrouter",
    secret: "OPENROUTER_KEY",
    url: "https://openrouter.ai/api/v1/chat/completions",
    model: "nvidia/nemotron-3-super-120b-a12b:free",
    weight: 4,
  },
  {
    id: "nvidia",
    secret: "NVIDIA_KEY",
    url: "https://integrate.api.nvidia.com/v1/chat/completions",
    model: "meta/llama-3.3-70b-instruct",
    // Measured at twenty seconds cold, so it is a last resort rather than a peer.
    weight: 5,
  },
];

/** Cloudflare's own models, reached through the binding rather than a key. */
const ON_EDGE = "@cf/qwen/qwen3-30b-a3b-fp8";

const SYSTEM = `You answer questions about ELDEN RING and its mods for a player who is
mid-game and wants a short answer.

Use only the wiki passages given to you. If they do not cover it, say so plainly
rather than guessing — a confident wrong answer about a boss or a build costs
somebody a run.

Be brief. Two or three sentences unless asked for more. No preamble.`;

function prompt(question, passages) {
  const context = (passages ?? [])
    .slice(0, 4)
    .map((p, i) => `[${i + 1}] ${p.title}\n${String(p.text).slice(0, 2400)}`)
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

/** One attempt at one provider. Returns the text, or null to move on. */
async function ask(provider, key, messages) {
  const started = Date.now();
  try {
    const res = await fetch(provider.url, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${key}` },
      body: JSON.stringify({ model: provider.model, messages, max_tokens: 500, temperature: 0.3 }),
      signal: AbortSignal.timeout(20000),
    });
    if (!res.ok) {
      return { ok: false, why: `${res.status} ${(await res.text()).slice(0, 120)}` };
    }
    const body = await res.json();
    const message = body.choices?.[0]?.message ?? {};
    // A reasoning model can leave `content` empty and put everything in its own
    // field, which looks like success and reads as silence.
    const text = (message.content || message.reasoning_content || "").trim();
    if (!text) return { ok: false, why: "empty answer" };
    return { ok: true, text, ms: Date.now() - started };
  } catch (error) {
    return { ok: false, why: error.message?.slice(0, 120) ?? "failed" };
  }
}

async function askEdge(env, messages) {
  const started = Date.now();
  try {
    const out = await env.AI.run(ON_EDGE, { messages, max_tokens: 500, temperature: 0.3 });
    const text = (out.response ?? "").trim();
    if (!text) return { ok: false, why: "empty answer" };
    return { ok: true, text, ms: Date.now() - started };
  } catch (error) {
    return { ok: false, why: error.message?.slice(0, 120) ?? "failed" };
  }
}

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "content-type",
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

    // Which providers this deployment can actually reach. Useful on its own,
    // and it is how the launcher tells a dead deployment from a dead network.
    if (url.pathname === "/health") {
      const ready = PROVIDERS.filter((p) => env[p.secret]).map((p) => p.id);
      return json({ ok: true, providers: [...ready, env.AI ? "cloudflare" : null].filter(Boolean) });
    }

    // Every provider, tried once, with what each one said. This is how the
    // geo-blocked ones get checked from the edge rather than from a desk in a
    // country they refuse.
    if (url.pathname === "/probe") {
      const messages = [{ role: "user", content: "Reply with exactly: ok" }];
      const results = {};
      for (const provider of PROVIDERS) {
        const key = env[provider.secret];
        results[provider.id] = key
          ? await ask(provider, key, messages)
          : { ok: false, why: "no key set" };
      }
      results.cloudflare = env.AI ? await askEdge(env, messages) : { ok: false, why: "no binding" };
      return json(results);
    }

    if (url.pathname !== "/ask" || request.method !== "POST") {
      return json({ error: "POST /ask" }, 404);
    }

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: "expected JSON" }, 400);
    }

    const question = String(body.question ?? "").trim().slice(0, 600);
    if (!question) return json({ error: "no question" }, 400);

    const messages = prompt(question, body.passages);
    const tried = [];

    for (const provider of PROVIDERS) {
      const key = env[provider.secret];
      if (!key) continue;
      const out = await ask(provider, key, messages);
      if (out.ok) {
        return json({ answer: out.text, provider: provider.id, ms: out.ms, tried });
      }
      tried.push({ provider: provider.id, why: out.why });
    }

    if (env.AI) {
      const out = await askEdge(env, messages);
      if (out.ok) return json({ answer: out.text, provider: "cloudflare", ms: out.ms, tried });
      tried.push({ provider: "cloudflare", why: out.why });
    }

    // Everything is out. Say which, rather than a bare failure — a day's
    // allowance running out and a broken key look identical otherwise.
    return json({ error: "every provider refused", tried }, 503);
  },
};
