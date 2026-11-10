// Stand-ins for markdown.js and code.js that parses.test.js gives case.ts:
// each passes through to the real one, and counts the calls in `parses`.
import * as md from "../static/markdown.ts";
import * as code from "../static/code.ts";

export const parses = { markdown: 0, inlineMarkdown: 0, codeBlock: 0 };

export function markdown(text) {
  parses.markdown++;
  return md.markdown(text);
}

export function inlineMarkdown(text) {
  parses.inlineMarkdown++;
  return md.inlineMarkdown(text);
}

export function codeBlock(text, diff, wrap) {
  parses.codeBlock++;
  return code.codeBlock(text, diff, wrap);
}

export const { isDiff, wrapped } = code;
