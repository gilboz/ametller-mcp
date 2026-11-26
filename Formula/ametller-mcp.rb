class AmetllerMcp < Formula
  desc "Local Ametller grocery catalog and cart MCP server"
  homepage "https://github.com/gilboz/ametller-mcp"
  url "https://github.com/gilboz/ametller-mcp.git", tag: "v0.1.0-beta.2"
  version "0.1.0-beta.2"
  license "MIT"

  depends_on "rust" => :build

  deny_network_access!

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      This private source repository requires GitHub access through Git.
      Sign in privately with: ametller-mcp login

      Add the Codex plugin:
        codex plugin marketplace add gilboz/ametller-mcp --ref main
        codex plugin add ametller@ametller-mcp

      Product search works as a guest. Cart access needs your own Ametller login.
      Cart changes do not place orders; review and pay on the official site.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ametller-mcp --version")
    capabilities = JSON.parse(shell_output("#{bin}/ametller-mcp capabilities"))
    assert_equal 5, capabilities.fetch("tools").length
    assert_includes capabilities.fetch("tools"), "set_cart_item"
  end
end
