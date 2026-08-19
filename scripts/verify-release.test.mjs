import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { verifyRelease } from "./verify-release.mjs";

const releasePath = new URL("../releases/devnet-2026-08-19-r8.json", import.meta.url);

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
