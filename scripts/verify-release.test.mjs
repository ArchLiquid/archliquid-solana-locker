import assert from "node:assert/strict";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { verifyRelease } from "./verify-release.mjs";

const releasePath = new URL("../releases/devnet-2026-08-19-r8.json", import.meta.url);
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

async function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "arch-locker-source-"));
  cpSync(repositoryRoot, directory, {
    recursive: true,
    filter: (source) => ![".git", "target", "node_modules"].includes(basename(source)),
  });
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const verifier = await import(pathToFileURL(join(directory, "scripts/verify-release.mjs")).href);
  return { directory, verify: verifier.verifyRelease };
}

test("verifies the immutable devnet release", () => {
  const result = verifyRelease();
  assert.equal(result.programId, "2jDqQUZY7yidwa8DTQm6vyFcptFPZ4QhxJn5ePGpgd2p");
});

test("rejects tampered release identity", () => {
  const manifest = JSON.parse(readFileSync(releasePath, "utf8"));
  manifest.program.executableSha256 = "0".repeat(64);
  const directory = mkdtempSync(join(tmpdir(), "arch-locker-release-"));
  const tamperedPath = join(directory, "release.json");
  writeFileSync(tamperedPath, `${JSON.stringify(manifest, null, 2)}\n`);
  assert.throws(() => verifyRelease(tamperedPath), /executable SHA-256/);
});

test("rejects changes to the preserved release-era bot configuration", async (t) => {
  const { directory, verify } = await fixture(t);
  writeFileSync(join(directory, "releases/devnet-2026-08-19-r8/operations/.github/dependabot.yml"), "version: 2\nupdates: []\n");
  assert.throws(() => verify(), /historical operational source SHA-256/);
});

test("still rejects changes to live program source", async (t) => {
  const { directory, verify } = await fixture(t);
  writeFileSync(join(directory, "programs/arch_locker/src/lib.rs"), "// substituted program\n");
  assert.throws(() => verify(), /source input SHA-256/);
});

test("still rejects changes to the pinned live CI workflow", async (t) => {
  const { directory, verify } = await fixture(t);
  writeFileSync(join(directory, ".github/workflows/ci.yml"), "name: substituted workflow\n");
  assert.throws(() => verify(), /source input SHA-256/);
});
