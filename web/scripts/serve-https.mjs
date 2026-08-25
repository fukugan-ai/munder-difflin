import { spawn } from "node:child_process";
import { once } from "node:events";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const web = resolve(repo, "web");
const dx = resolve(repo, ".task-tools/dioxus/bin/dx");
const configuredTarget = process.env.CARGO_TARGET_DIR || "target";
const target = isAbsolute(configuredTarget)
  ? configuredTarget
  : resolve(web, configuredTarget);

async function run(command, args, options) {
  const child = spawn(command, args, { stdio: "inherit", ...options });
  const forward = (signal) => child.kill(signal);
  process.once("SIGINT", forward);
  process.once("SIGTERM", forward);
  const [code, signal] = await once(child, "exit");
  process.removeListener("SIGINT", forward);
  process.removeListener("SIGTERM", forward);
  if (signal) return 128;
  return code ?? 1;
}

const buildCode = await run(
  dx,
  ["build", "--web", "--fullstack", "true", "-p", "md-web-app", "--cargo-args=--locked"],
  { cwd: web, env: process.env },
);
if (buildCode !== 0) {
  process.exitCode = buildCode;
} else {
  const executable = resolve(
    target,
    "dx/md-web-app/debug/web",
    process.platform === "win32" ? "server.exe" : "server",
  );
  process.exitCode = await run(executable, [], {
    cwd: dirname(executable),
    env: {
      ...process.env,
      MD_WEB_HTTPS: "true",
      IP: process.env.IP || "0.0.0.0",
      PORT: process.env.PORT || "5080",
    },
  });
}
