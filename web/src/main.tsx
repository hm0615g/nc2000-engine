import { render } from "preact";
import { App } from "./app";
import { readDoor } from "./info-mode";
import { Evaluate } from "./evaluate";
import { Fork } from "./fork";
import { KifuTool } from "./kifu";
import { initTooltips } from "./tooltip";
import { initAnnouncer } from "./announcer";
import "./style.css";

initTooltips();
initAnnouncer();
const door = readDoor();
const params = new URLSearchParams(location.search);
const forkName = params.get("fork")?.trim();
const legacyFork = forkName && !["1", "true"].includes(forkName.toLowerCase());
render(
  params.has("kifu-preview") || (door === "fork" && !legacyFork && !params.has("advanced")) ? <KifuTool /> :
    door === "evaluate" ? <Evaluate /> : door === "fork" ? <Fork /> : <App />,
  document.getElementById("app")!,
);
