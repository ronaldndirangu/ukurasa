// Downloads the pinned PDFium build from bblanchon/pdfium-binaries into src-tauri/pdfium.
// Set UKURASA_SKIP_PDFIUM=1 to skip; pass --force to download again.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

// Must match the PDFium version targeted by the pdfium-render `pdfium_latest` feature.
const RELEASE = "chromium/8125";

const ASSETS = {
  "win32-x64": ["pdfium-win-x64.tgz", "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b"],
  "win32-arm64": ["pdfium-win-arm64.tgz", "d394ba61991fb53f4d87f70988035f337d9e73f2bd2c3c1b966df8c43fcabb05"],
  "darwin-x64": ["pdfium-mac-x64.tgz", "8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0"],
  "darwin-arm64": ["pdfium-mac-arm64.tgz", "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d"],
  "linux-x64": ["pdfium-linux-x64.tgz", "e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659"],
  "linux-arm64": ["pdfium-linux-arm64.tgz", "8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa"],
};

const LIBRARY = {
  win32: "bin/pdfium.dll",
  darwin: "lib/libpdfium.dylib",
  linux: "lib/libpdfium.so",
};

const RETRY_DELAYS_MS = [2000, 4000, 8000];
const TIMEOUT_MS = 120_000;

const dest = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "pdfium");
const stampFile = join(dest, "RELEASE.txt");

async function main() {
  if (process.env.UKURASA_SKIP_PDFIUM === "1") {
    console.log("pdfium: skipped (UKURASA_SKIP_PDFIUM=1)");
    return;
  }

  const target = `${process.platform}-${process.arch}`;
  const asset = ASSETS[target];
  if (!asset) {
    console.warn(`pdfium: no prebuilt PDFium for ${target}; set UKURASA_PDFIUM_PATH to a local build`);
    return;
  }
  const [name, sha256] = asset;
  const stamp = `${RELEASE} ${name}\n`;

  if (!process.argv.includes("--force") && existsSync(stampFile) && readFileSync(stampFile, "utf8") === stamp) {
    console.log(`pdfium: ${RELEASE} already present`);
    return;
  }

  const url = `https://github.com/bblanchon/pdfium-binaries/releases/download/${encodeURIComponent(RELEASE)}/${name}`;
  console.log(`pdfium: downloading ${url}`);
  const archive = await download(url);

  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== sha256) {
    throw new Error(`checksum mismatch for ${name}: expected ${sha256}, got ${actual}`);
  }

  rmSync(dest, { recursive: true, force: true });
  mkdirSync(join(dest, "licenses"), { recursive: true });

  const library = LIBRARY[process.platform];
  let foundLibrary = false;
  for (const entry of tarEntries(gunzipSync(archive))) {
    if (entry.name === library) {
      writeFileSync(join(dest, library.split("/").pop()), entry.body);
      foundLibrary = true;
    } else if (entry.name === "LICENSE" || /^licenses\/[\w.+-]+$/.test(entry.name)) {
      writeFileSync(join(dest, entry.name), entry.body);
    }
  }
  if (!foundLibrary) {
    throw new Error(`${library} not found in ${name}`);
  }

  writeFileSync(stampFile, stamp);
  console.log(`pdfium: installed ${RELEASE} into ${dest}`);
}

/** Retries dropped connections, timeouts, rate limits, and server errors, which are usually brief. */
async function download(url) {
  // GitHub limits anonymous downloads more strictly, so CI sends its workflow token.
  const token = process.env.GITHUB_TOKEN;
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  for (let attempt = 0; ; attempt++) {
    // Zero means no response arrived or its body broke off.
    let status = 0;
    let reason;
    try {
      const response = await fetch(url, { headers, signal: AbortSignal.timeout(TIMEOUT_MS) });
      if (response.ok) {
        return Buffer.from(await response.arrayBuffer());
      }
      status = response.status;
      reason = `HTTP ${status}`;
    } catch (error) {
      reason = error.cause?.code ?? error.message;
    }
    const retryable = status === 0 || status === 408 || status === 429 || status >= 500;
    if (!retryable || attempt === RETRY_DELAYS_MS.length) {
      throw new Error(`download failed: ${reason}`);
    }
    const delay = RETRY_DELAYS_MS[attempt];
    console.warn(`pdfium: download failed (${reason}); retrying in ${delay / 1000} s`);
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
}

function* tarEntries(tar) {
  let offset = 0;
  let longName = null;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      return;
    }
    const field = (start, length) =>
      header.subarray(start, start + length).toString("utf8").replace(/\0[\s\S]*$/, "");
    const size = parseInt(field(124, 12).trim() || "0", 8);
    const type = field(156, 1) || "0";
    const prefix = field(257, 6) === "ustar" ? field(345, 155) : "";
    const name = longName ?? (prefix ? `${prefix}/${field(0, 100)}` : field(0, 100));
    longName = null;

    const body = tar.subarray(offset + 512, offset + 512 + size);
    offset += 512 + Math.ceil(size / 512) * 512;

    if (type === "L") {
      longName = body.toString("utf8").replace(/\0[\s\S]*$/, "");
    } else if (type === "x") {
      longName = /\d+ path=([^\n]*)\n/.exec(body.toString("utf8"))?.[1] ?? null;
    } else if (type === "0") {
      yield { name: name.replace(/^\.\//, ""), body };
    }
  }
}

main().catch((error) => {
  console.error(`pdfium: ${error.message}`);
  console.error("pdfium: set UKURASA_SKIP_PDFIUM=1 to skip, or UKURASA_PDFIUM_PATH to use a local build");
  process.exit(1);
});
