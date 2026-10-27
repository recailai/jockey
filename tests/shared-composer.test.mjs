import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "vite";
import { handleCompletionKeyDown } from "../src/lib/completionKeyboard.ts";

test("completion keyboard behavior is shared by both composers", () => {
  let slashIndex = 0;
  let selected;
  let closed = false;
  const completions = {
    slashOpen: () => true,
    slashItems: () => [{ value: "/model" }, { value: "/mode" }],
    slashActiveIndex: () => slashIndex,
    mentionOpen: () => false,
    mentionItems: () => [],
    mentionActiveIndex: () => 0,
    _setSlashActiveIndex: (fn) => { slashIndex = fn(slashIndex); },
    _setMentionActiveIndex: () => {},
    applySlashCandidate: (item) => { selected = item.value; },
    applyMentionCandidate: () => {},
    closeSlashMenu: () => { closed = true; },
    closeMentionMenu: () => {},
  };
  const event = (key) => ({ key, prevented: false, preventDefault() { this.prevented = true; } });
  const down = event("ArrowDown");
  assert.equal(handleCompletionKeyDown(down, completions), true);
  assert.equal(down.prevented, true);
  assert.equal(slashIndex, 1);
  handleCompletionKeyDown(event("Enter"), completions);
  assert.equal(selected, "/mode");
  handleCompletionKeyDown(event("Escape"), completions);
  assert.equal(closed, true);
  assert.equal(handleCompletionKeyDown(event("x"), completions), false);
});

test("a stale persona lookup cannot overwrite a newer selection or another session", async (t) => {
  const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
  t.after(() => server.close());
  const { useSessionAgentSelection } = await server.ssrLoadModule("/src/hooks/useSessionAgentSelection.ts");
  const { projectAgentApi } = await server.ssrLoadModule("/src/lib/tauriApi.ts");
  const pending = new Map();
  projectAgentApi.getConfig = (_project, role) => new Promise((resolve) => pending.set(role, resolve));
  let current = { id: "s1", activeRole: "Developer", runtimeKind: "claude-code", projectId: "p1" };
  const updates = [];
  const sessions = {
    activeSession: () => current,
    activeSessionId: () => current.id,
    patchActiveSession: (patch) => { updates.push(patch); Object.assign(current, patch); },
    updateSession: () => {},
  };
  const agents = {
    roles: () => [{ roleName: "Reviewer", runtimeKind: "codex-cli" }, { roleName: "Planner", runtimeKind: "dsh-cli" }],
    assistants: () => [{ key: "dsh-cli", profileId: "dsh" }],
    fetchConfigOptions: async () => [],
  };
  const selection = useSessionAgentSelection(sessions, agents, (message) => assert.fail(message));
  const old = selection.selectRole("Reviewer");
  const latest = selection.selectRole("Planner");
  pending.get("Planner")({ runtimeKind: null });
  await latest;
  pending.get("Reviewer")({ runtimeKind: "codex-cli" });
  await old;
  assert.equal(updates.length, 1);
  assert.equal(current.activeRole, "Planner");
  assert.equal(current.runtimeKind, "dsh-cli");
  assert.equal(current.runtimeProfileId, "dsh");
  const other = selection.selectRole("Reviewer");
  current = { ...current, id: "s2" };
  pending.get("Reviewer")({ runtimeKind: "codex-cli" });
  await other;
  assert.equal(updates.length, 1);
});

test("persisted persona changes use the engine's saved binding and serialize selections", async (t) => {
  const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
  t.after(() => server.close());
  const { useSessionAgentSelection } = await server.ssrLoadModule("/src/hooks/useSessionAgentSelection.ts");
  const { appSessionApi } = await server.ssrLoadModule("/src/lib/tauriApi.ts");
  const requests = [];
  const pending = [];
  appSessionApi.selectRole = (id, role) => new Promise((resolve) => {
    requests.push([id, role]);
    pending.push(resolve);
  });
  const current = { id: "s1", persisted: true, activeRole: "Developer", runtimeKind: "claude-code" };
  const sessions = {
    activeSession: () => current,
    activeSessionId: () => current.id,
    patchActiveSession: () => assert.fail("persisted selection must be saved by the engine"),
    updateSession: (_id, patch) => Object.assign(current, patch),
  };
  const agents = { roles: () => [], assistants: () => [], fetchConfigOptions: async () => [] };
  const selection = useSessionAgentSelection(sessions, agents, (message) => assert.fail(message));
  const first = selection.selectRole("Reviewer");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const second = selection.selectRole("Planner");
  assert.deepEqual(requests, [["s1", "Reviewer"]]);
  pending.shift()({ activeRole: "Reviewer", runtimeKind: "codex-cli", runtimeProfileId: null });
  await first;
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(requests, [["s1", "Reviewer"], ["s1", "Planner"]]);
  pending.shift()({ activeRole: "Planner", runtimeKind: "dsh-cli", runtimeProfileId: "bound-profile" });
  await second;
  assert.equal(current.activeRole, "Planner");
  assert.equal(current.runtimeKind, "dsh-cli");
  assert.equal(current.runtimeProfileId, "bound-profile");
});
