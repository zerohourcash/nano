import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repository = path.resolve(root, "..", "..");
const git = args => {
  const result = spawnSync("git", args, { cwd: repository, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(result.stderr.trim() || `git ${args.join(" ")} failed`);
  return result.stdout.trim();
};
const revision = git(["rev-parse", "--verify", "HEAD"]);
const dirty = git(["status", "--porcelain", "--untracked-files=no"]) !== "";
const buildRevision = `${revision}${dirty ? "-dirty" : ""}`;
const target = path.join(root, ".build", "cargo");
const result = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--manifest-path",
    path.join(root, "backend", "Cargo.toml"),
    "--target-dir",
    target,
  ],
  {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, EVERYDAY_BUILD_REVISION: buildRevision },
  }
);

if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

const filename =
  process.platform === "win32" ? "meshkeeper-node.exe" : "meshkeeper-node";
const source = path.join(target, "release", filename);
if (!existsSync(source)) throw new Error(`Cargo did not produce ${source}`);
const output = path.join(root, "dist", "server");
mkdirSync(output, { recursive: true });
const destination = path.join(output, filename);
const staged = path.join(output, `.${filename}.next-${process.pid}`);
rmSync(staged, { force: true });
try {
  copyFileSync(source, staged);
  chmodSync(staged, statSync(source).mode);
  // POSIX rename replaces the directory entry atomically even while the old
  // inode is executing. Existing processes finish on the old binary; the next
  // controlled restart opens the fully copied release instead of observing a
  // partial executable or failing with ETXTBSY.
  renameSync(staged, destination);
} finally {
  rmSync(staged, { force: true });
}
