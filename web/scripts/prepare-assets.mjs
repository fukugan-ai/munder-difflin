import { copyFile, mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const vendor = resolve(repo, "web/app/assets/vendor");
await mkdir(vendor, { recursive: true });
const fontSource = resolve(repo, "node_modules/@fontsource/noto-sans-jp");
await Promise.all([
  copyFile(resolve(repo, "node_modules/@xterm/xterm/lib/xterm.js"), resolve(vendor, "xterm.js")),
  copyFile(resolve(repo, "node_modules/@xterm/xterm/css/xterm.css"), resolve(vendor, "xterm.css")),
  copyFile(resolve(repo, "node_modules/@xterm/addon-fit/lib/addon-fit.js"), resolve(vendor, "addon-fit.js")),
  copyFile(
    resolve(repo, "node_modules/@xterm/addon-unicode11/lib/addon-unicode11.js"),
    resolve(vendor, "addon-unicode11.js"),
  ),
  copyFile(resolve(fontSource, "LICENSE"), resolve(vendor, "noto-sans-jp.LICENSE")),
  ...[400, 600, 700].map((weight) =>
    copyFile(
      resolve(fontSource, `files/noto-sans-jp-japanese-${weight}-normal.woff2`),
      resolve(vendor, `noto-sans-jp-japanese-${weight}-normal.woff2`),
    ),
  ),
  writeFile(
    resolve(vendor, "noto-sans-jp.css"),
    [400, 600, 700]
      .map(
        (weight) => `@font-face {
  font-family: "Noto Sans JP";
  font-style: normal;
  font-display: swap;
  font-weight: ${weight};
  src: url("./noto-sans-jp-japanese-${weight}-normal.woff2") format("woff2");
}`,
      )
      .join("\n\n"),
    "utf8",
  ),
]);
