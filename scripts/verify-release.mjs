import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const repositoryRoot = realpathSync(join(dirname(scriptPath), ".."));
const defaultManifest = join(repositoryRoot, "releases/devnet-2026-08-19-r8.json");
const programId = "2jDqQUZY7yidwa8DTQm6vyFcptFPZ4QhxJn5ePGpgd2p";
const executableSha256 =
  "effd9de765f2bbb66f3f01ea9bb767304981891ceb30b8ce4f063e326e8e5b03";
const idlSha256 =
  "8eee74c0f3802b269d159bd78b8acab7781bf50500424364ccfd77e6db3d5f52";

function fail(message) {
  throw new Error(message);
}

function exact(actual, expected, label) {
  if (actual !== expected) fail(`${label} must be ${JSON.stringify(expected)}`);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function checkedPath(candidate, label) {
  if (isAbsolute(candidate)) fail(`${label} path must be relative`);
  if (/(?:keypair|secret|private-key|id\.json|\.env)/i.test(candidate)) {
    fail(`${label} path must not reference key material`);
  }
  const path = resolve(repositoryRoot, candidate);
  if (path !== repositoryRoot && !path.startsWith(`${repositoryRoot}${sep}`)) {
    fail(`${label} path escapes the repository`);
  }
  return path;
}

function verifyEntry(entry, label) {
  const path = checkedPath(entry.path, label);
  exact(sha256(path), entry.sha256, `${label} SHA-256`);
}

export function verifyRelease(manifestPath = defaultManifest) {
  const path = resolve(manifestPath);
  const manifest = JSON.parse(readFileSync(path, "utf8"));

  exact(manifest.schemaVersion, 1, "schemaVersion");
  exact(manifest.status, "immutable-devnet-release", "status");
  exact(manifest.qualityStandard, "production", "qualityStandard");
  exact(manifest.mainnetApproved, false, "mainnetApproved");
  exact(manifest.realValueCustodyApproved, false, "realValueCustodyApproved");
  exact(manifest.externalAuditClaimed, false, "externalAuditClaimed");
  exact(manifest.program.programId, programId, "program ID");
  exact(manifest.program.upgradeAuthority, null, "upgrade authority");
  exact(manifest.program.programDataAuthorityOption, 0, "ProgramData authority option");
  exact(manifest.program.finalizationFinalized, true, "finalization status");
  exact(manifest.program.executableSha256, executableSha256, "executable SHA-256");
  exact(manifest.program.idlSha256, idlSha256, "IDL SHA-256");

  for (const entry of manifest.sourceInputs) verifyEntry(entry, "source input");
  verifyEntry(manifest.idl, "IDL");

  const binaryPath = join(repositoryRoot, "target/deploy/arch_locker.so");
  if (existsSync(binaryPath)) {
    exact(sha256(binaryPath), executableSha256, "built executable SHA-256");
  }

  return { programId, executableSha256 };
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  if (process.argv.length > 3) fail("usage: node scripts/verify-release.mjs [manifest]");
  const result = verifyRelease(process.argv[2]);
  console.log(`Verified immutable ArchLocker devnet release ${result.programId}.`);
  console.log(`Executable SHA-256: ${result.executableSha256}`);
}
