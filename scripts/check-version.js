const fs = require("node:fs");
const path = require("node:path");

function checkVersion(root, releaseTag) {
  const npm = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  const lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
  const tauri = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
  const cargo = fs.readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8");
  const packageSection = cargo.split(/^\[build-dependencies\]/m)[0];
  const cargoName = packageSection.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
  const cargoVersion = packageSection.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  const versions = [npm.version, lock.version, lock.packages?.[""]?.version, tauri.version, cargoVersion];
  const names = [npm.name, lock.name, lock.packages?.[""]?.name, cargoName];
  if (versions.some((version) => version !== cargoVersion)) {
    throw new Error(`버전 불일치: npm/lock/root-lock/Tauri/Cargo = ${versions.join(" / ")}`);
  }
  if (names.some((name) => name !== cargoName)) {
    throw new Error(`패키지 이름 불일치: npm/lock/root-lock/Cargo = ${names.join(" / ")}`);
  }
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(cargoVersion)) {
    throw new Error(`지원하지 않는 버전 문자열: ${cargoVersion}`);
  }
  if (releaseTag && releaseTag !== `v${cargoVersion}`) {
    throw new Error(`릴리스 태그 ${releaseTag}가 제품 버전 v${cargoVersion}와 다릅니다`);
  }
  return cargoVersion;
}

if (require.main === module) {
  try {
    const version = checkVersion(path.resolve(__dirname, ".."), process.env.RELEASE_TAG || process.argv[2]);
    console.log(`버전 확인 완료: v${version}`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { checkVersion };
