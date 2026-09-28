#!/usr/bin/env node

"use strict";

const path = require("node:path");
const { spawn } = require("node:child_process");

const packages = new Map([
  ["linux:x64", "@convesoft/work-linux-x64-gnu"],
  ["linux:arm64", "@convesoft/work-linux-arm64-gnu"],
  ["darwin:arm64", "@convesoft/work-darwin-arm64"],
]);

const packageName = packages.get(`${process.platform}:${process.arch}`);
if (packageName === undefined) {
  console.error(`Work has no binary for ${process.platform}/${process.arch}.`);
  process.exitCode = 1;
} else {
  let binary;
  try {
    binary = path.join(path.dirname(require.resolve(`${packageName}/package.json`)), "bin", "work");
  } catch {
    console.error(`Missing optional package ${packageName}. Reinstall @convesoft/work with optional dependencies enabled.`);
    process.exitCode = 1;
  }

  if (binary !== undefined) {
    const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });
    const handlers = new Map();
    for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
      const handler = () => child.kill(signal);
      handlers.set(signal, handler);
      process.on(signal, handler);
    }
    child.once("error", (error) => {
      console.error(`Could not start ${packageName}: ${error.message}`);
      process.exitCode = 1;
    });
    child.once("exit", (code, signal) => {
      for (const [name, handler] of handlers) process.off(name, handler);
      if (signal !== null) process.kill(process.pid, signal);
      else process.exitCode = code ?? 1;
    });
  }
}
