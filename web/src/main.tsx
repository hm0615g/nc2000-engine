import { render } from "preact";
import { App } from "./app";
import { readDoor } from "./info-mode";
import { Evaluate } from "./evaluate";
import { Fork } from "./fork";
import { initTooltips } from "./tooltip";
import { initAnnouncer } from "./announcer";
import "./style.css";

initTooltips();
initAnnouncer();
const door = readDoor();
render(
  door === "evaluate" ? <Evaluate /> : door === "fork" ? <Fork /> : <App />,
  document.getElementById("app")!,
);
