#!/usr/bin/env node

import { chmod, copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const targets = new Map([
  ["x86_64-unknown-linux-gnu", { name: "@convesoft/work-linux-x64-gnu", os: ["linux"], cpu: ["x64"], libc: ["glibc"] }],
  ["aarch64-unknown-linux-gnu", { name: "@convesoft/work-linux-arm64-gnu", os: ["linux"], cpu: ["arm64"], libc: ["glibc"] }],
  ["aarch64-apple-darwin", { name: "@convesoft/work-darwin-arm64", os: ["darwin"], cpu: ["arm64"] }],
]);

const cargo = await readFile(path.join(root, "Cargo.toml"), "utf8");
const packageSection = cargo.match(/\[package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? "";
const version = packageSection.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const license = packageSection.match(/^license\s*=\s*"([^"]+)"/m)?.[1];
if (!version || version === "0.0.0" || !license) {
  throw new Error("release version and license must be selected in Cargo.toml");
}

function manifest(name, description) {
  return {
    name, version, description, license,
    repository: { type: "git", url: "git+https://github.com/convesoft/work.git" },
    publishConfig: { access: "public" },
  };
}

async function destination(output, name) {
  const directory = path.join(path.resolve(output), name.replace("@convesoft/", ""));
  await rm(directory, { recursive: true, force: true });
  await mkdir(path.join(directory, "bin"), { recursive: true });
  await Promise.all(["README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"].map((file) => copyFile(path.join(root, file), path.join(directory, file))));
  return directory;
}

async function writeManifest(directory, value) {
  await writeFile(path.join(directory, "package.json"), `${JSON.stringify(value, null, 2)}\n`);
  process.stdout.write(`${directory}\n`);
}

const [command, ...args] = process.argv.slice(2);
if (command === "main" && args.length === 1) {
  const name = "@convesoft/work";
  const directory = await destination(args[0], name);
  await copyFile(path.join(root, "npm/work.cjs"), path.join(directory, "bin/work.cjs"));
  await chmod(path.join(directory, "bin/work.cjs"), 0o755);
  await writeManifest(directory, {
    ...manifest(name, "Local, agent-first issue tracker"),
    bin: { work: "bin/work.cjs" },
    engines: { node: ">=18" },
    optionalDependencies: Object.fromEntries([...targets.values()].map(({ name }) => [name, version])),
    files: ["bin/work.cjs", "README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"],
  });
} else if (command === "platform" && args.length === 3) {
  const [target, binary, output] = args;
  const config = targets.get(target);
  if (!config) throw new Error(`unsupported Rust target: ${target}`);
  const directory = await destination(output, config.name);
  await copyFile(path.resolve(binary), path.join(directory, "bin/work"));
  await chmod(path.join(directory, "bin/work"), 0o755);
  await writeManifest(directory, {
    ...manifest(config.name, `Work native binary for ${target}`),
    os: config.os, cpu: config.cpu,
    ...(config.libc ? { libc: config.libc } : {}),
    files: ["bin/work", "README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"],
  });
} else {
  console.error("usage: package-npm.mjs main <output-dir> | platform <target> <binary> <output-dir>");
  process.exitCode = 2;
}
