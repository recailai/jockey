import { render } from "solid-js/web";
import "../index.css";
import "../styles/design-system.css";
import { trackInputModality } from "../lib/inputModality";
import "./companion.css";
import { setTransport } from "../lib/transport";
import CompanionApp from "./CompanionApp";
import { createHttpTransport, readStoredToken, type HttpTransport } from "./httpTransport";

document.documentElement.classList.add("companion-host");
trackInputModality();
const syncViewport = () => {
  document.documentElement.style.setProperty("--companion-viewport-height", `${window.visualViewport?.height ?? window.innerHeight}px`);
};
syncViewport();
window.visualViewport?.addEventListener("resize", syncViewport);
window.addEventListener("resize", syncViewport);

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
