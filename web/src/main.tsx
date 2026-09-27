import { render } from "preact";
import { App } from "./app";
import { readDoor } from "./info-mode";
import { Evaluate } from "./evaluate";
import { initTooltips } from "./tooltip";
import { initAnnouncer } from "./announcer";
import "./style.css";

initTooltips();
initAnnouncer();
render(
  readDoor() === "evaluate" ? <Evaluate /> : <App />,
  document.getElementById("app")!,
);
