import "./lib/browserMock";
import { lazy } from "solid-js";
import { render } from "solid-js/web";
import App from "./App";
import "./index.css";

// The UI catalog is a dev-only page; production builds tree-shake this branch away.
const Catalog = import.meta.env.DEV ? lazy(() => import("./components/catalog/Catalog")) : null;
const showCatalog = Catalog !== null && window.location.hash === "#catalog";

render(() => (showCatalog && Catalog ? <Catalog /> : <App />), document.getElementById("root") as HTMLElement);
