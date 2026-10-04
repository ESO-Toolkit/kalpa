#!/usr/bin/env node

import { readFileSync } from "node:fs";
const readJson = (name) =>
  JSON.parse(readFileSync(new URL(`../${name}`, import.meta.url), "utf8"));

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const lockedRootVersion = packageLock.packages?.[""]?.version;
const expected = "0.0.0";
const violations = [];

if (packageJson.version !== expected) {
  violations.push(`package.json version = ${JSON.stringify(packageJson.version)}`);
}
if (packageLock.version !== expected) {
  violations.push(`package-lock.json version = ${JSON.stringify(packageLock.version)}`);
}
if (lockedRootVersion !== expected) {
  violations.push(
    `package-lock.json packages[\"\"].version = ${JSON.stringify(lockedRootVersion)}`
  );
}

if (violations.length > 0) {
  console.error("Worker version policy failed:");
  for (const violation of violations) console.error(`  - ${violation}`);
  console.error(
    `The private Worker deploys independently from desktop release tags; keep all package metadata at ${expected}.`
  );
  process.exit(1);
}

console.log(
  `Worker version policy passed: private independently deployed package uses ${expected}.`
);
