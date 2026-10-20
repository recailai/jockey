import { render } from "solid-js/web";
import "../index.css";
import "./companion.css";
import { setTransport } from "../lib/transport";
import CompanionApp from "./CompanionApp";
import { createHttpTransport, readStoredToken, type HttpTransport } from "./httpTransport";

async function createTransport(): Promise<HttpTransport> {
  // `?mock` serves recorded fixtures through the same seam; it is not part of production builds.
  if (import.meta.env.DEV && new URLSearchParams(window.location.search).has("mock")) {
    const { createMockTransport } = await import("./mockTransport");
    return createMockTransport();
  }
  return createHttpTransport(readStoredToken());
}

void createTransport().then((transport) => {
  setTransport(transport);
  render(() => <CompanionApp transport={transport} />, document.getElementById("root") as HTMLElement);
});
