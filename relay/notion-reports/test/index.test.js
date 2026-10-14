import { test } from "node:test";
import assert from "node:assert/strict";
import worker, { validate, notionPage, LIMITS } from "../src/index.js";

const good = { kind: "bug", title: " Crash ", description: "It crashed.", version: "0.20.0", os: "windows x86_64" };

test("validate trims and keeps a good report", () => {
  const r = validate(good);
  assert.equal(r.title, "Crash");
  assert.equal(r.log, "");
});

test("validate rejects bad reports", () => {
  assert.match(validate(null), /Invalid/);
  assert.match(validate({ ...good, kind: "spam" }), /kind/);
  assert.match(validate({ ...good, title: "  " }), /title/);
  assert.match(validate({ ...good, description: "" }), /description/);
  assert.match(validate({ ...good, title: "x".repeat(LIMITS.title + 1) }), /too long/);
});

test("the ticket lands in Backlog with its type and chunked text", () => {
  const page = notionPage(validate({ ...good, kind: "idea", log: "l".repeat(4500) }), "ds");
  assert.deepEqual(page.parent, { type: "data_source_id", data_source_id: "ds" });
  assert.equal(page.properties.Statut.select.name, "Backlog");
  assert.equal(page.properties.Type.select.name, "Fonctionnalité");
  assert.equal(page.properties.Nom.title[0].text.content, "Crash");
  const code = page.children.find((b) => b.type === "code");
  assert.equal(code.code.rich_text.length, 3);
  assert.ok(code.code.rich_text.every((t) => t.text.content.length <= 2000));
});

test("fetch forwards a valid report to Notion", async () => {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response("{}", { status: 200 });
  };
  const env = { NOTION_TOKEN: "secret", NOTION_DATA_SOURCE_ID: "ds" };
  const post = (path, body) =>
    worker.fetch(new Request(`https://w.dev${path}`, { method: "POST", body: JSON.stringify(body) }), env);

  assert.equal((await post("/report", good)).status, 201);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].init.headers.Authorization, "Bearer secret");

  assert.equal((await post("/report", { ...good, kind: "x" })).status, 400);
  assert.equal((await post("/other", good)).status, 404);
  assert.equal(calls.length, 1, "invalid reports never reach Notion");
});

test("rate limited senders are refused", async () => {
  const env = { RATE_LIMITER: { limit: async () => ({ success: false }) } };
  const req = new Request("https://w.dev/report", { method: "POST", body: JSON.stringify(good) });
  assert.equal((await worker.fetch(req, env)).status, 429);
});
