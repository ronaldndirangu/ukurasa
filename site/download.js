const REPO = "ronaldndirangu/ukurasa";

function detectOs() {
  const platform = (navigator.userAgentData?.platform || navigator.platform || "").toLowerCase();
  const agent = navigator.userAgent.toLowerCase();
  if (/android|iphone|ipad/.test(agent)) return null;
  if (platform.startsWith("win")) return "windows";
  if (platform.startsWith("mac")) return "mac";
  if (platform.includes("linux")) return "linux";
  return null;
}

// Only Chromium reports the CPU; Safari and Firefox fall back to Apple silicon.
async function isIntelMac() {
  try {
    const { architecture } = await navigator.userAgentData.getHighEntropyValues(["architecture"]);
    return architecture === "x86";
  } catch {
    return false;
  }
}

function formatSize(bytes) {
  return `${Math.round(bytes / 1e6)} MB`;
}

async function linkAssets() {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!response.ok) return;
  const release = await response.json();

  for (const link of document.querySelectorAll("[data-asset]")) {
    const asset = release.assets.find((a) => a.name.endsWith(link.dataset.asset));
    if (!asset) {
      link.hidden = true;
      continue;
    }
    link.href = asset.browser_download_url;
    const size = document.createElement("span");
    size.className = "size";
    size.textContent = formatSize(asset.size);
    link.append(" ", size);
  }

  const version = release.tag_name.replace(/^v/, "");
  const date = new Date(release.published_at).toLocaleDateString(undefined, { dateStyle: "long" });
  document.getElementById("release-line").textContent =
    `Version ${version}, released ${date}. Built from the public source on GitHub.`;
  document.getElementById("hero-note").textContent = `Version ${version} for Windows, macOS, and Linux.`;
}

async function main() {
  const assets = linkAssets().catch(() => {});
  const os = detectOs();
  if (!os) return;

  const card = document.querySelector(`[data-os="${os}"]`);
  card.classList.add("current");
  card.parentElement.prepend(card);

  if (os === "mac" && (await isIntelMac())) {
    const [silicon, intel] = card.querySelectorAll("[data-asset]");
    intel.after(silicon);
    intel.className = silicon.className;
    silicon.className = "alt";
  }

  const hero = document.getElementById("hero-download");
  const label = { windows: "Windows", mac: "Mac", linux: "Linux" }[os];
  hero.textContent = `Download for ${label}`;

  await assets;
  const best = card.querySelector(".button.primary[data-asset]:not([hidden])");
  hero.href = best ? best.href : "#download";
  if (best && os === "linux") hero.textContent = "Download for Linux (.deb)";
}

main();
