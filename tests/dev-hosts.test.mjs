import assert from "node:assert/strict";
import { test } from "node:test";
import { networkInterfaces } from "node:os";
import { request } from "node:http";
import { createServer, loadConfigFromFile } from "vite";

const get = (port, host) => new Promise((resolve, reject) => {
  const req = request({ hostname: "127.0.0.1", port, path: "/", headers: { Host: host } }, (res) => {
    res.resume();
    res.on("end", () => resolve(res.statusCode));
  });
  req.on("error", reject);
  req.end();
});

test("dev host allowlist accepts configured hosts and LAN IPs while rejecting forged domains", async (t) => {
  const previous = process.env.JOCKEY_ALLOWED_HOSTS;
  process.env.JOCKEY_ALLOWED_HOSTS = " jockey.test, , preview.jockey.test ";
  t.after(() => {
    if (previous === undefined) delete process.env.JOCKEY_ALLOWED_HOSTS;
    else process.env.JOCKEY_ALLOWED_HOSTS = previous;
  });
  const loaded = await loadConfigFromFile({ command: "serve", mode: "development" });
  assert.deepEqual(loaded.config.server.allowedHosts, ["jockey.test", "preview.jockey.test"]);
  const server = await createServer({
    ...loaded.config,
    configFile: false,
    server: { ...loaded.config.server, host: "127.0.0.1", port: 0, strictPort: false, hmr: false, watch: null },
  });
  t.after(() => server.close());
  await server.listen();
  const port = server.httpServer.address().port;
  assert.equal(await get(port, "forged.example"), 403);
  assert.equal(await get(port, "jockey.test"), 200);
  assert.equal(await get(port, "jockey.test.attacker.example"), 403);
  const lan = Object.values(networkInterfaces()).flat().find((entry) => entry.family === "IPv4" && !entry.internal);
  assert.ok(lan, "a real LAN interface is required for the IP regression");
  assert.equal(await get(port, `${lan.address}:${port}`), 200);
});
