import assert from "node:assert/strict";
import { test } from "node:test";
import { webcrypto } from "node:crypto";
import { createClientId } from "../src/lib/clientId.ts";
import { createHttpTransport } from "../src/companion/httpTransport.ts";
import { responseMessage, responseSession } from "../src/lib/messagePresentation.ts";
import { allowOnceOption } from "../src/lib/permissionOptions.ts";

function replaceGlobal(t, name, value) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, { configurable: true, value });
  t.after(() => {
    if (previous) Object.defineProperty(globalThis, name, previous);
    else delete globalThis[name];
  });
}

test("partner presentation keeps responses and decisions but removes process detail", () => {
  const base = { id: "a1", roleName: "Developer", text: "Final answer", at: 1 };
  for (const messageType of ["thought", "tool", "event", "context"]) {
    assert.equal(responseMessage({ ...base, messageType }), null);
  }
  for (const purpose of ["providerEvent", "roleHandoff", "taskState"]) {
    assert.equal(responseMessage({ ...base, purpose }), null);
  }
  const mixed = { ...base, purpose: "response", thoughtText: "private thought", toolCalls: [{ title: "Read file" }], segments: [{ kind: "text", text: "Interim commentary" }, { kind: "thought", text: "Thinking" }] };
  const projected = responseMessage(mixed);
  assert.equal(projected.text, "Final answer");
  assert.equal(projected.segments, undefined);
  assert.equal(projected.thoughtText, undefined);
  assert.equal(mixed.segments.length, 2);
  const permission = { requestId: "p1" };
  const question = { requestId: "q1" };
  const session = { messages: [mixed, { ...base, id: "live" }], submitting: true, streamingMessage: { id: "live", roleName: "Developer", text: "Reading the store so far" }, toolCalls: { t1: { toolCallId: "t1", title: "Read file", status: "completed" }, t2: { toolCallId: "t2", title: "Run tests", status: "running" } }, pendingPermissions: [permission], pendingUserInput: [question], thoughtText: "Thinking", streamSegments: [{}], currentPlan: [{}], usage: { costUsd: 3 } };
  const view = responseSession(session);
  assert.equal(view.messages.length, 1, "the live checkpoint row is shown once, as the stream");
  assert.equal(view.streamingMessage.text, "Reading the store so far", "a running turn keeps its streamed reply visible");
  assert.deepEqual(view.toolCalls, {});
  assert.deepEqual(view.streamSegments, []);
  assert.equal(view.agentState, "Run tests…");
  assert.equal(responseSession({ ...session, submitting: false }).streamingMessage, null);
  assert.equal(view.usage, null);
  assert.equal(view.currentPlan, null);
  assert.deepEqual(view.pendingPermissions, [permission]);
  assert.deepEqual(view.pendingUserInput, [question]);
  assert.equal(responseMessage({ ...base, roleName: "event", payload: { kind: "turnError", code: "AUTH_REQUIRED" } }).payload.code, "AUTH_REQUIRED");
});

test("batch approval selects only explicit one-time permission options", () => {
  assert.equal(allowOnceOption([{ optionId: "always", kind: "allow_always" }]), undefined);
  assert.equal(allowOnceOption([{ optionId: "allow_once", kind: "allow_always" }]), undefined);
  assert.equal(allowOnceOption([{ optionId: "arbitrary", title: "Allow" }]), undefined);
  const once = { optionId: "provider-specific-once", kind: "allow_once" };
  assert.equal(allowOnceOption([{ optionId: "always", kind: "allow_always" }, once]), once);
  assert.equal(allowOnceOption([{ optionId: "allow_once" }]).optionId, "allow_once");
});

test("HTTP clients can generate distinct UUIDs without the secure-context API", (t) => {
  let entropyRequests = 0;
  replaceGlobal(t, "crypto", {
    getRandomValues(bytes) {
      entropyRequests++;
      return webcrypto.getRandomValues(bytes);
    },
  });
  const ids = Array.from({ length: 1000 }, createClientId);
  assert.equal(new Set(ids).size, ids.length);
  assert.equal(entropyRequests, ids.length);
  for (const id of ids) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

test("HTTP transport preserves message ids and input across the engine boundary", async (t) => {
  const requests = [];
  t.mock.method(globalThis, "fetch", async (url, init) => {
    requests.push({ url, ...init });
    return Response.json({ status: "started" });
  });
  const transport = createHttpTransport("test-token");
  const input = { sessionId: "s1", text: "hello", userMessageId: "u1", assistantMessageId: "a1", runtimeKind: "codex-cli" };
  assert.deepEqual(await transport.call("submit_input", { input }), { status: "started" });
  assert.equal(requests[0].url, "/api/sessions/s1/chat");
  assert.equal(requests[0].headers.Authorization, "Bearer test-token");
  assert.deepEqual(JSON.parse(requests[0].body), { ...input, input: "hello" });
});

test("SSE closes on logout and when the last consumer unmounts", async (t) => {
  const streams = [];
  class FakeEventSource {
    closed = false;
    constructor(url) { this.url = url; streams.push(this); }
    addEventListener() {}
    close() { this.closed = true; }
  }
  replaceGlobal(t, "EventSource", FakeEventSource);
  replaceGlobal(t, "window", { localStorage: { removeItem() {} } });
  t.mock.method(globalThis, "fetch", async () => Response.json({ error: "unauthorized" }, { status: 401 }));
  const transport = createHttpTransport("expired-token");
  let unauthorized = 0;
  transport.onUnauthorized(() => unauthorized++);
  const off = await transport.listen("turn/started", () => {});
  await assert.rejects(transport.call("list_projects_cmd"), /Token/);
  assert.equal(unauthorized, 1);
  assert.equal(transport.hasToken(), false);
  assert.equal(streams[0].closed, true);
  off();
  const other = createHttpTransport("valid-token");
  const offOther = await other.listen("turn/started", () => {});
  offOther();
  assert.equal(streams[1].closed, true);
});

test("HTTP session operations retain runtime selection, updates, imports and full history", async (t) => {
  const requests = [];
  const imported = { sessions: [{ id: "imported", messages: [{ roleName: "user", text: "history" }] }], createdProjects: [{ id: "new-project" }] };
  t.mock.method(globalThis, "fetch", async (url, init) => {
    requests.push({ url, body: init.body && JSON.parse(init.body) });
    return Response.json(url === "/api/import" ? imported : { id: "s1" });
  });
  const transport = createHttpTransport("test-token");
  const creation = { title: "Session_1", projectId: "p1", runtimeKind: "codex-cli", runtimeProfileId: "codex", id: "client-id" };
  await transport.call("create_app_session", creation);
  assert.deepEqual(requests.at(-1), { url: "/api/sessions", body: creation });
  const update = { runtimeKind: "pi-cli", runtimeProfileId: "pi", title: "Renamed" };
  await transport.call("update_app_session", { id: "s1", update });
  assert.deepEqual(requests.at(-1), { url: "/api/sessions/s1/update", body: update });
  await transport.call("update_app_session", { id: "s1", update: { activeRole: "Reviewer" } });
  assert.deepEqual(requests.at(-1), { url: "/api/sessions/s1/role", body: { role: "Reviewer" } });
  assert.deepEqual(await transport.call("import_cli_sessions_cmd", { sourceIds: ["source"] }), imported);
  await transport.call("get_app_sessions", { ids: ["s1"] });
  assert.equal(requests.at(-1).url, "/api/sessions/s1");
});
