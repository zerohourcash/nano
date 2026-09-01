import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const platform = { win32: "windows", darwin: "macos", linux: "linux" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "arm64" }[process.arch] ?? process.arch;
const manifestPath = path.join(
  root,
  ".release",
  `everyday-${platform}-${arch}`,
  "release-manifest.json"
);
const original = readFileSync(manifestPath, "utf8");

try {
  const tampered = JSON.parse(original);
  tampered.sourceRevision = "0".repeat(40);
  writeFileSync(manifestPath, `${JSON.stringify(tampered, null, 2)}\n`);
  const rejected = spawnSync(
    process.execPath,
    [path.join(root, "scripts", "verify-desktop-package.mjs")],
    {
      cwd: root,
      env: process.env,
      encoding: "utf8",
    }
  );
  if (
    rejected.status === 0 ||
    !`${rejected.stderr}${rejected.stdout}`.includes("source revision")
  ) {
    throw new Error("Desktop verifier accepted a forged source revision");
  }
} finally {
  writeFileSync(manifestPath, original);
}

console.log("Desktop provenance tamper test passed");
