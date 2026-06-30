# Security Policy / 安全策略

> English follows Chinese. / 中文在前,英文在后。

---

## 支持的版本 / Supported Versions

| Source / 来源                | Supported / 支持 |
| ---------------------------- | ---------------- |
| Latest release / 最新 release | ✅               |
| Older releases / 旧 release   | ❌               |

**仅最新 release 版本接收安全修复。** 请始终升级到最新版本,旧版本不再维护。

**Only the latest release is supported.** Please always upgrade to the latest version; older releases are no longer maintained.

---

## 报告漏洞 / Reporting a Vulnerability

### 中文

**请勿通过公开 GitHub issue 报告安全漏洞。** 公开讨论会让攻击者在修复发布前抢先利用。

请使用 **GitHub 私有漏洞报告**(推荐,加密传输、可直接转成安全公告):

  👉 <https://github.com/kokifish/Asig/security/advisories/new>

或发送邮件至 **k0k1fish@outlook.com**(标题请加 `[Asig Security]` 前缀)。

**响应时间**:通常在 **72 小时**内确认收到;7 天内给出初步评估和修复计划。

**请在报告中附上**:

- 漏洞描述与影响范围
- 复现步骤 / PoC(代码、截图或录像)
- 受影响版本与运行环境(macOS 版本、Asig 版本)
- 您的联系方式(供跟进)

### English

**Please do not report security vulnerabilities through public GitHub issues.** Public disclosure lets attackers exploit issues before a fix ships.

Use **GitHub private vulnerability reporting** (recommended; encrypted, can be turned into an advisory directly):

  👉 <https://github.com/kokifish/Asig/security/advisories/new>

Or email **k0k1fish@outlook.com** (please prefix the subject with `[Asig Security]`).

**Response time**: confirmation within **72 hours**; initial triage and fix plan within **7 days**.

**Please include**:

- Vulnerability description and impact
- Reproduction steps / PoC (code, screenshot, or video)
- Affected versions and environment (macOS version, Asig version)
- Your contact info (for follow-up)

---

## 披露策略 / Disclosure Policy

- 我们采用**协调披露**:确认漏洞后,会先发布修复版本,再公开 CVE / GHSA 详情。
- 修复发布后,致谢报告者(除非你要求匿名)。
- 若 **90 天**内未收到回复,可在公开渠道自行披露 —— 但仍建议先邮件沟通。

- We follow **coordinated disclosure**: fix is released first, then full details (CVE / GHSA) are published.
- Reporters are credited after the fix ships (unless anonymity is requested).
- If we have not responded within **90 days**, you may disclose publicly — but please email us first.

---

## 范围 / Scope

In-scope: `crates/core`、`crates/app`、`scripts/make-app.sh`、Release 产物(`.app`)。
Out-of-scope: 第三方 agent(Claude Code / CodeBuddy / OpenClaw / Trae)自身的漏洞,以及运行 Asig 的 macOS 系统本身。

In-scope: `crates/core`, `crates/app`, `scripts/make-app.sh`, release artifacts (`.app`).
Out-of-scope: bugs in third-party agents (Claude Code / CodeBuddy / OpenClaw / Trae) and in macOS itself.

---

## 致谢 / Acknowledgements

感谢所有负责任地报告安全问题的人。

Thanks to everyone who reports security issues responsibly.
