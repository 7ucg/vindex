/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import "./styles.css";
import { applyAll } from "./lib/theme";

applyAll();

const root = document.getElementById("root");
if (root) render(() => <App />, root);
