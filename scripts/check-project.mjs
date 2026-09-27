import assert from "node:assert/strict";
import { readFileSync, readdirSync, appendFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const readJson = (path) => JSON.parse(readFileSync(resolve(projectRoot, path), "utf8"));

export function parseVersion(value) {
  assert.equal(typeof value, "string", "版本号必须是字符串");
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(value);
  assert.ok(match, `不是有效的 SemVer 版本: ${value}`);
  if (match[4]) {
    for (const part of match[4].split(".")) {
      assert.ok(!/^0\d+$/.test(part), `预发布数字不能包含前导零: ${value}`);
    }
  }
  return { version: value, numbers: match.slice(1, 4).map(BigInt), prerelease: Boolean(match[4]) };
}

export function compareStableVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  assert.ok(!a.prerelease && !b.prerelease, "稳定入口只能比较正式版本");
  for (let i = 0; i < 3; i++) {
    if (a.numbers[i] !== b.numbers[i]) return a.numbers[i] > b.numbers[i] ? 1 : -1;
  }
  return 0;
}

function tomlField(section, name) {
  const match = new RegExp(`^${name}\\s*=\\s*"([^"\\r\\n]+)"\\s*$`, "m").exec(section);
  assert.ok(match, `TOML 缺少 ${name}`);
  return match[1];
}

export function checkProject(tag = undefined) {
  const pkg = readJson("package.json");
  const lock = readJson("package-lock.json");
  const config = readJson("src-tauri/tauri.conf.json");
  const cargo = readFileSync(resolve(projectRoot, "src-tauri/Cargo.toml"), "utf8");
  const cargoPackage = cargo.split(/^\[package\]\s*$/m)[1]?.split(/^\[/m)[0];
  assert.ok(cargoPackage, "Cargo.toml 缺少 package 节");
  const cargoName = tomlField(cargoPackage, "name");
  const cargoLock = readFileSync(resolve(projectRoot, "src-tauri/Cargo.lock"), "utf8")
    .split(/^\[\[package\]\]\s*$/m)
    .slice(1)
    .filter((entry) => tomlField(entry, "name") === cargoName);
  assert.equal(cargoLock.length, 1, "Cargo.lock 中必须只有一个应用包");
  const version = parseVersion(pkg.version);
  const versions = {
    "package-lock.json": lock.version,
    "package-lock.json packages.root": lock.packages?.[""]?.version,
    "Cargo.toml": tomlField(cargoPackage, "version"),
    "Cargo.lock": tomlField(cargoLock[0], "version"),
    "tauri.conf.json": config.version,
  };
  for (const [file, value] of Object.entries(versions)) {
    assert.equal(value, version.version, `${file} 版本与 package.json 不一致`);
  }
  if (tag !== undefined) {
    assert.equal(tag, `v${version.version}`, "Release tag 必须与源码及锁文件版本完全一致");
  }
  assert.equal(config.bundle?.createUpdaterArtifacts, true, "发布必须生成签名更新产物");
  assert.deepEqual(config.bundle?.targets, ["nsis"], "当前发布校验仅支持一个 NSIS 安装器");
  assert.ok(config.plugins?.updater?.pubkey?.trim(), "缺少更新公钥");
  assert.ok(config.plugins.updater.endpoints?.length, "缺少更新入口");
  for (const endpoint of config.plugins.updater.endpoints) {
    assert.equal(new URL(endpoint).protocol, "https:", "更新入口必须使用 HTTPS");
  }
  assert.ok(config.app?.security?.csp, "本地界面必须显式配置 CSP");
  const capabilityDir = resolve(projectRoot, "src-tauri/capabilities");
  for (const file of readdirSync(capabilityDir).filter((name) => name.endsWith(".json"))) {
    const capability = readJson(`src-tauri/capabilities/${file}`);
    if (capability.remote?.urls?.length) {
      assert.equal(capability.permissions?.length ?? 0, 0, `${file} 不应向远程文档授权本地 IPC`);
    }
    if (capability.permissions?.length) {
      assert.ok(!capability.windows?.length, `${file} 应按本地 WebView 授权而非整个窗口`);
      assert.ok(capability.webviews?.length && capability.webviews.every((label) => ["ui", "settings"].includes(label)), `${file} 只能向 ui/settings 授权`);
      for (const permission of capability.permissions) {
        const name = typeof permission === "string" ? permission : permission.identifier;
        assert.ok(!["core:default", "core:event:default", "core:event:allow-emit", "core:event:allow-emit-to"].includes(name), `${file} 不应开放通用事件写权限`);
      }
    }
  }
  return version;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  assert.ok(args.length === 0 || (args.length === 2 && args[0] === "--tag"), "用法: node scripts/check-project.mjs [--tag v1.2.3]");
  const version = checkProject(args[1] ?? process.env.RELEASE_TAG);
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `version=${version.version}\nprerelease=${version.prerelease}\n`);
  }
  console.log(`版本、锁文件与发布配置检查通过: ${version.version}${version.prerelease ? " (prerelease)" : " (stable)"}`);
}
