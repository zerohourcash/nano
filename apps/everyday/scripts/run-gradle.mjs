import { existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const android = path.join(root, "android");
const windows = process.platform === "win32";
const wrapper = path.join(android, windows ? "gradlew.bat" : "gradlew");
const args = process.argv.slice(2);

if (args.length === 0) {
  console.error("Usage: node scripts/run-gradle.mjs <task> [arguments]");
  process.exit(2);
}
if (!existsSync(wrapper)) {
  console.error(`Pinned Gradle wrapper is missing: ${wrapper}`);
  process.exit(2);
}

const env = { ...process.env };
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
env.EVERYDAY_BUILD_REVISION = `${revision}${dirty ? "-dirty" : ""}`;
// CI/dev images commonly install the SDK here. Explicit environment variables
// always win, so production builders can use any SDK location.
if (
  !env.ANDROID_HOME &&
  !env.ANDROID_SDK_ROOT &&
  existsSync("/opt/android-sdk")
) {
  env.ANDROID_HOME = "/opt/android-sdk";
  env.ANDROID_SDK_ROOT = "/opt/android-sdk";
}

const result = spawnSync(wrapper, args, {
  cwd: android,
  env,
  stdio: "inherit",
  shell: windows,
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
