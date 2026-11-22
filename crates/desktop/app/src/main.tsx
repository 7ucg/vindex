/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import "./styles.css";
import { applyTheme, getTheme } from "./lib/theme";

applyTheme(getTheme());

const root = document.getElementById("root");
if (root) render(() => <App />, root);
