import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { tmpdir } from "node:os";
import { checkProject, readJson, compareStableVersions } from "./check-project.mjs";
import { verifyInstaller } from "./verify-signature.mjs";

const [mode] = process.argv.slice(2);
assert.ok(["prepare", "manifest", "publish"].includes(mode), "用法: node scripts/release.mjs prepare|manifest|publish");
assert.equal(process.argv.length, 3, "不接受其他命令行参数");
const tag = process.env.RELEASE_TAG;
assert.ok(tag, "缺少 RELEASE_TAG");
const version = checkProject(tag);
const repo = process.env.GITHUB_REPOSITORY;
assert.ok(/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo ?? ""), "缺少或无效的 GITHUB_REPOSITORY");
assert.ok(process.env.GH_TOKEN, "缺少 GH_TOKEN");
const config = readJson("src-tauri/tauri.conf.json");
const releasePath = `repos/${repo}/releases/tags/${encodeURIComponent(tag)}`;

function gh(args, binaryLimit) {
  return execFileSync("gh", args, {
    encoding: binaryLimit === undefined ? "utf8" : "buffer",
    maxBuffer: binaryLimit ?? 4 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
}

function api(path, optional = false) {
  // 用状态码区分不存在与鉴权/网络失败；后者不得被当作首次发布。
  try {
    const response = gh(["api", "--include", path]);
    const boundary = /\r?\n\r?\n/.exec(response);
    assert.ok(boundary, "GitHub API 响应缺少 HTTP 头");
    return JSON.parse(response.slice(boundary.index + boundary[0].length));
  } catch (error) {
    if (optional && /^HTTP\/\S+ 404\b/m.test(String(error.stdout ?? ""))) return null;
    throw error;
  }
}

function draftRelease() {
  // 按 tag 获取发布的 API 不保证返回草稿；使用构建步骤明确返回的 release ID。
  const id = process.env.RELEASE_ID;
  assert.ok(/^[1-9]\d*$/.test(id ?? ""), "缺少有效的构建输出 RELEASE_ID");
  const release = api(`repos/${repo}/releases/${id}`);
  assert.equal(release.tag_name, tag, "Release tag 不一致");
  assert.equal(release.draft, true, "只能修改草稿；已发布版本不允许重跑覆盖");
  return release;
}

function releaseFiles(release, withManifest) {
  const assets = release.assets;
  assert.ok(Array.isArray(assets), "Release assets 无效");
  const installers = assets.filter((asset) => asset.name.endsWith(".exe"));
  assert.equal(installers.length, 1, "Release 必须且只能有一个 NSIS 安装器");
  const installer = installers[0];
  assert.ok(installer.name.endsWith(`_${version.version}_x64-setup.exe`), "安装器名称与版本/架构不符");
  assert.ok(installer.size <= 128 * 1024 * 1024, "安装器超过客户端 128 MiB 限制");
  const assetByName = (name) => {
    const matches = assets.filter((asset) => asset.name === name);
    assert.equal(matches.length, 1, `缺少或重复的资源: ${name}`);
    assert.equal(matches[0].state, "uploaded", `资源未完成上传: ${name}`);
    assert.ok(matches[0].size > 0, `空资源: ${name}`);
    return matches[0];
  };
  const selected = [assetByName(installer.name), assetByName(`${installer.name}.sig`)];
  assert.ok(selected[1].size <= 16 * 1024, "签名资源过大");
  if (withManifest) selected.push(assetByName("latest.json"));
  if (withManifest) assert.ok(selected[2].size <= 64 * 1024, "更新清单过大");
  const directory = mkdtempSync(join(resolve(process.env.RUNNER_TEMP ?? tmpdir()), "feishu-release-"));
  for (const asset of selected) {
    assert.ok(!asset.name.includes("/") && !asset.name.includes("\\"), "资源名不能包含路径");
    const downloaded = gh(["api", `repos/${repo}/releases/assets/${asset.id}`, "--header", "Accept: application/octet-stream"], asset.size + 1024);
    assert.equal(downloaded.length, asset.size, `资源长度不匹配: ${asset.name}`);
    writeFileSync(join(directory, asset.name), downloaded);
  }
  const signature = readFileSync(join(directory, `${installer.name}.sig`), "utf8").trim();
  const bytes = readFileSync(join(directory, installer.name));
  verifyInstaller(bytes, signature, config.plugins.updater.pubkey, version.version);
  assert.ok(bytes.subarray(0, 2).equals(Buffer.from("MZ")) && bytes.includes(Buffer.from("efbeadde4e756c6c736f6674496e7374", "hex")), "安装包不是客户端支持的 NSIS 格式");
  const url = `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(installer.name)}`;
  return { directory, signature, url };
}

if (mode === "prepare") {
  const existing = api(releasePath, true);
  if (existing) assert.equal(existing.draft, true, "已发布版本禁止覆盖；请提高版本号");
  if (!version.prerelease) {
    const latest = api(`repos/${repo}/releases/latest`, true);
    if (latest) {
      assert.equal(latest.prerelease, false, "当前 latest 不是稳定版本");
      assert.ok(latest.tag_name.startsWith("v"), "当前 latest tag 不符合版本约定");
      assert.ok(compareStableVersions(version.version, latest.tag_name.slice(1)) > 0, "不能用旧版本覆盖稳定更新入口");
    }
  }
  console.log(`发布前检查通过: ${tag}，${version.prerelease ? "预发布" : "稳定版"}`);
} else if (mode === "manifest") {
  const files = releaseFiles(draftRelease(), false);
  const manifest = {
    version: version.version,
    notes: `自动发布 ${tag}`,
    pub_date: new Date().toISOString(),
    platforms: { "windows-x86_64": { signature: files.signature, url: files.url } },
  };
  const path = join(files.directory, "latest.json");
  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  gh(["release", "upload", tag, path, "--repo", repo, "--clobber"]);
  console.log(`${tag} 安装包及可信版本验签通过，清单已上传草稿`);
} else {
  const release = draftRelease();
  const files = releaseFiles(release, true);
  const manifest = JSON.parse(readFileSync(join(files.directory, "latest.json"), "utf8"));
  assert.equal(manifest.version, version.version, "更新清单版本不一致");
  assert.ok(Number.isFinite(Date.parse(manifest.pub_date)), "更新清单日期无效");
  assert.deepEqual(Object.keys(manifest.platforms), ["windows-x86_64"], "更新清单平台不符");
  assert.deepEqual(manifest.platforms["windows-x86_64"], { signature: files.signature, url: files.url }, "更新清单必须指向刚刚验证的安装器及签名");
  if (!version.prerelease) {
    const latest = api(`repos/${repo}/releases/latest`, true);
    if (latest) {
      assert.ok(latest.tag_name.startsWith("v"), "当前 latest tag 不符合版本约定");
      assert.ok(compareStableVersions(version.version, latest.tag_name.slice(1)) > 0, "已有更新的稳定版本，保留本次草稿");
    }
  }
  const publishPath = join(files.directory, "publish.json");
  writeFileSync(publishPath, JSON.stringify({ draft: false, prerelease: version.prerelease, make_latest: version.prerelease ? "false" : "true" }));
  // 最后一步单次 PATCH：检查全部完成后才公开，预发布永远不切换 latest。
  gh(["api", "--method", "PATCH", `repos/${repo}/releases/${release.id}`, "--input", publishPath]);
  console.log(`${tag} 已公开，${version.prerelease ? "稳定更新入口保持不变" : "已更新稳定入口"}`);
}
