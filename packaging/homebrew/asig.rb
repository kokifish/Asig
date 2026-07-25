# Asig Homebrew formula(自建 tap 用)。
#
# 路线:源码编译(非预编译)→ 产物天然无 com.apple.quarantine → 免 Gatekeeper,
# 免 Apple Developer($99)签名/公证。代价:首次安装需编译(~30s,依赖 rust)。
#
# 发布步骤(见同目录 README.md):
#   1. 建 github.com/kokifish/homebrew-asig 仓库
#   2. 把本文件放进去(Formula/asig.rb)
#   3. 用户:brew tap kokifish/asig && brew install asig
class Asig < Formula
  desc "macOS menubar light that monitors Claude Code / OpenClaw / Hermes agent status"
  homepage "https://github.com/kokifish/Asig"
  license "MIT"

  # 稳定版:发首个 GitHub Release(tag,如 v0.1.0)后启用下面两行。
  # 算 sha256:curl -sL https://github.com/kokifish/Asig/archive/refs/tags/v0.1.0.tar.gz | shasum -a 256
  # url "https://github.com/kokifish/Asig/archive/refs/tags/v0.1.0.tar.gz"
  # sha256 "________________________________"
  # version "0.1.0"

  # 当前未发 Release → 走 HEAD(从 main 分支源码编译):
  #   brew install --HEAD kokifish/asig
  head "https://github.com/kokifish/Asig.git", branch: "main"

  depends_on "rust" => :build

  def install
    # make-app.sh:cargo build --release + 组装 Asig.app(LSUIElement 菜单栏 accessory)。
    # 它自定位 repo 根(cd "$(dirname "$0")/.."),在此源码树里直接调用即可。
    system "./scripts/make-app.sh"
    # 装 .app 到 prefix。Homebrew formula 不自动放 /Applications(那是 cask 的 appdir),
    # 故 caveats 指引用户 open 或软链。产物无 quarantine,双击/启动均不被 Gatekeeper 拦。
    prefix.install "build/Asig.app"
  end

  def caveats
    <<~EOS
      Asig.app 装在:  #{prefix}/Asig.app
      启动:          open #{prefix}/Asig.app
      软链到 /Applications(可选):
                     ln -sf #{prefix}/Asig.app /Applications/Asig.app

      开机自启:启动后 Settings → General → Launch at login(SMAppService,macOS 13+)。
    EOS
  end

  test do
    assert_predicate prefix/"Asig.app", :exist?
  end
end
