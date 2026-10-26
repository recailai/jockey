import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "vite";

test("first send persists the draft persona despite the default-role creation event", async (t) => {
  const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
  t.after(() => server.close());
  const { useSessionManager } = await server.ssrLoadModule("/src/hooks/useSessionManager.ts");
  const { makeDraftSession } = await server.ssrLoadModule("/src/lib/sessionHelpers.ts");
  const { appSessionApi } = await server.ssrLoadModule("/src/lib/tauriApi.ts");
  const manager = useSessionManager();
  const draft = makeDraftSession("Persona", { activeRole: "Reviewer", runtimeKind: "dsh-cli" });
  manager.setSessions([draft]);
  let saved;
  appSessionApi.create = async () => {
    manager.updateSession(draft.id, { activeRole: "Developer", runtimeKind: "claude-code" });
    return { id: draft.id, title: "Persona" };
  };
  appSessionApi.update = async (id, patch) => { saved = { id, ...patch }; };
  await manager.ensureSessionPersisted(draft.id);
  assert.equal(saved.id, draft.id);
  assert.equal(saved.activeRole, "Reviewer");
  assert.equal(saved.runtimeKind, "dsh-cli");
  assert.equal(manager.sessions[0].activeRole, "Reviewer");
  assert.equal(manager.sessions[0].runtimeKind, "dsh-cli");
  assert.equal(manager.sessions[0].persisted, true);
  const { useAppBootstrap } = await server.ssrLoadModule("/src/hooks/useAppBootstrap.ts");
  const restored = { id: draft.id, title: "Persona", activeRole: "Reviewer", runtimeKind: "dsh-cli", messages: [] };
  appSessionApi.list = async () => [restored];
  appSessionApi.listSummaries = async () => [];
  appSessionApi.listInbox = async () => [];
  saved = null;
  let loaded;
  const { bootstrapApp } = useAppBootstrap({
    setSessions: (rows) => { if (Array.isArray(rows)) loaded = rows; },
    setActiveSessionId: () => {},
    assistants: () => [{ key: "claude-code", available: true }],
    roles: () => [{ roleName: "Reviewer", runtimeKind: "claude-code" }],
    refreshAssistants: async () => {},
    refreshRoles: async () => {},
    refreshSkills: async () => {},
    showToast: (message) => assert.fail(message),
  });
  await bootstrapApp();
  assert.equal(loaded[0].activeRole, "Reviewer");
  assert.equal(loaded[0].runtimeKind, "dsh-cli");
  assert.equal(saved, null, "restoring must not rewrite the selected engine to an available default");

});
