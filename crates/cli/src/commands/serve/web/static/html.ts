// HTML-like templates in tagged strings, drawn by Preact: no JSX, no build.
import { h } from "preact";
import htm from "htm";

export const html = htm.bind(h);
