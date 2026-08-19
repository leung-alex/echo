import { createRoot } from "react-dom/client";

import { EchoApp } from "./app/EchoApp";
import "./echo-ui.css";
import "./features/clipboard/clipboard.css";
import "./features/settings/settings.css";

const root = document.querySelector<HTMLDivElement>("#app");
if (!root) throw new Error("Echo app root is missing");
createRoot(root).render(<EchoApp />);
