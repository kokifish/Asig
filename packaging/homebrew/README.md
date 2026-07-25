# Homebrew Tap 发布指南

Asig 走**零成本分发**:不签名/不公证(省 $99 Apple Developer),靠 Homebrew **formula 源码编译**绕开 Gatekeeper —— 本地 `rustc` 产物天然无 `com.apple.quarantine`,双击即用。

> 为什么是 formula 不是 cask:官方 homebrew-cask 要求签名公证,进不去;自建 tap 的 formula 源码编译产物无隔离属性,干净通过 Gatekeeper。

## 一次性:建 tap 仓库

```bash
# 1. 在 GitHub 建 空仓库: kokifish/homebrew-asig (必须叫 homebrew-<name>)
# 2. 把本目录的 asig.rb 放进去,路径 Formula/asig.rb
git clone https://github.com/kokifish/homebrew-asig.git
mkdir -p Formula
cp /path/to/Asig/packaging/homebrew/asig.rb Formula/asig.rb
git add Formula/asig.rb && git commit -m "Add asig formula" && git push
```

## 用户安装

```bash
brew tap kokifish/asig
brew install --HEAD asig          # 当前(未发 Release,从 main 源码编译)
# 发 Release 后(asig.rb 填好 url+sha256):
# brew install asig                # 装稳定版
```

装在 `$(brew --prefix)/Cellar/asig/.../Asig.app`(`caveats` 会打印路径),`open` 即用。

## 发首个 Release 后的更新

1. 在 Asig 仓库打 tag:`git tag v0.1.0 && git push origin v0.1.0`,发 GitHub Release。
2. 算 tarball sha256:
   ```bash
   curl -sL https://github.com/kokifish/Asig/archive/refs/tags/v0.1.0.tar.gz | shasum -a 256
   ```
3. 编辑 `Formula/asig.rb`:取消 `url`/`sha256`/`version` 三行注释,填入 hash。
4. push tap 仓库。用户 `brew upgrade asig` 即得稳定版。

## 与 install.sh 的关系

| 方式 | 命令 | 适合 |
|---|---|---|
| install.sh | `curl -fsSL .../install.sh \| bash` | 不用 Homebrew 的用户 |
| Homebrew | `brew install asig` | Homebrew 用户(开发者主流) |

两者都零成本、产物无 quarantine。Homebrew 还顺带管理升级(`brew upgrade`)。
