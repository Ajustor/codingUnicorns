// Relay between the IDE's "Report an Issue" dialog and the Notion tickets
// board. It keeps the Notion token server side, checks every report and
// rate-limits senders, then creates a ticket in Backlog.
//
// Secrets / vars (see wrangler.toml and README.md):
//   NOTION_TOKEN           internal integration token (secret)
//   NOTION_DATA_SOURCE_ID  data source of "Coding Unicorns — Tickets"
//   RATE_LIMITER           optional rate limiting binding

const NOTION_VERSION = "2025-09-03";
const MAX_BODY_BYTES = 64 * 1024;
export const LIMITS = { title: 120, description: 8000, log: 20000, version: 32, os: 64 };
const KINDS = { bug: "Bug", idea: "Fonctionnalité" };
// Notion caps one rich text item at 2000 characters.
const CHUNK = 2000;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/report") return json({ error: "Not found." }, 404);
    if (request.method !== "POST") return json({ error: "Use POST." }, 405);

    if (env.RATE_LIMITER) {
      const key = request.headers.get("CF-Connecting-IP") || "unknown";
      const { success } = await env.RATE_LIMITER.limit({ key });
      if (!success) return json({ error: "Too many reports, try again in a minute." }, 429);
    }

    const length = Number(request.headers.get("Content-Length") || 0);
    if (length > MAX_BODY_BYTES) return json({ error: "Report too large." }, 413);
    let body;
    try {
      const text = await request.text();
      if (text.length > MAX_BODY_BYTES) return json({ error: "Report too large." }, 413);
      body = JSON.parse(text);
    } catch {
      return json({ error: "Invalid JSON." }, 400);
    }

    const report = validate(body);
    if (typeof report === "string") return json({ error: report }, 400);

    const res = await fetch("https://api.notion.com/v1/pages", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${env.NOTION_TOKEN}`,
        "Notion-Version": NOTION_VERSION,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(notionPage(report, env.NOTION_DATA_SOURCE_ID)),
    });
    if (!res.ok) {
      console.error("Notion error", res.status, await res.text());
      return json({ error: "The report could not be saved, try again later." }, 502);
    }
    return json({ ok: true }, 201);
  },
};

/// The report in `body`, trimmed, or a message saying what is wrong.
export function validate(body) {
  if (!body || typeof body !== "object") return "Invalid report.";
  const str = (v) => (typeof v === "string" ? v.trim() : "");
  const report = {
    kind: str(body.kind),
    title: str(body.title),
    description: str(body.description),
    version: str(body.version),
    os: str(body.os),
    log: str(body.log),
  };
  if (!KINDS[report.kind]) return "Unknown report kind.";
  if (!report.title) return "A title is required.";
  if (!report.description) return "A description is required.";
  for (const [field, max] of Object.entries(LIMITS)) {
    if (report[field].length > max) return `The ${field} is too long (${max} characters max).`;
  }
  return report;
}

/// The Notion "create page" request for `report`: a Backlog ticket whose
/// body holds the description, the app version and OS, and the log.
export function notionPage(report, dataSourceId) {
  const children = [
    callout(`Reported from the app · version ${report.version || "?"} · ${report.os || "?"}`),
    ...chunks(report.description).map((c) => block("paragraph", c)),
  ];
  if (report.log) {
    children.push(block("heading_3", "Launch log"));
    children.push({
      object: "block",
      type: "code",
      code: { language: "plain text", rich_text: chunks(report.log).map(text) },
    });
  }
  return {
    parent: { type: "data_source_id", data_source_id: dataSourceId },
    properties: {
      Nom: { title: [text(report.title)] },
      Statut: { select: { name: "Backlog" } },
      Type: { select: { name: KINDS[report.kind] } },
      // `Version` is left alone: it names the release holding the fix, set
      // by hand. The reporter's version is in the callout.
    },
    children,
  };
}

function chunks(s) {
  const out = [];
  for (let i = 0; i < s.length; i += CHUNK) out.push(s.slice(i, i + CHUNK));
  return out;
}

function text(content) {
  return { type: "text", text: { content } };
}

function block(type, content) {
  return { object: "block", type, [type]: { rich_text: [text(content)] } };
}

function callout(content) {
  return {
    object: "block",
    type: "callout",
    callout: { rich_text: [text(content)], icon: { type: "emoji", emoji: "🦄" } },
  };
}

function json(data, status) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
