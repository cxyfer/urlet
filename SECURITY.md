# Security policy

**English** | [繁體中文](docs/zh-TW/SECURITY.md)

urlet is a self-hosted service: every instance is run by its own operator, so a vulnerability
stays exploitable until operators upgrade. Please report security issues privately, allow time
for a fixed release to ship, and only then disclose the details.

## Contents

- [Disclaimer](#disclaimer)
- [Supported versions](#supported-versions)
- [Reporting a vulnerability](#reporting-a-vulnerability)
- [What happens next](#what-happens-next)
- [Scope](#scope)

## Disclaimer

urlet is a personal project, developed and maintained by one person in their spare time. It is
not a commercial product, and there is no support contract or service-level agreement.

- The software is provided "as is", without warranty of any kind, as stated in the MIT
  [`LICENSE`](LICENSE). The author is not liable for any damage, data loss or security incident
  arising from its use.
- Everything in this policy is best effort. A report may take longer than the goals below, and
  some issues may be fixed late, partially or not at all.
- Operators are responsible for their own instances: upgrading, securing the host and reverse
  proxy, backing up data, and deciding whether urlet fits their needs. Review it yourself before
  relying on it for high-risk or compliance-sensitive workloads.

## Supported versions

Security fixes land on `master` and ship in a new patch release of the newest stable minor
version. Older minor versions and `-rc.N` pre-releases do not receive backports; upgrade to the
latest release to get a fix.

| Version | Supported |
| --- | --- |
| Latest stable minor (currently `1.4.x`) | Yes |
| Older minor versions | No |
| Pre-releases (`-rc.N`) | No |

## Reporting a vulnerability

**Do not open a public issue, pull request or discussion for a security problem.**

Report it through GitHub's private vulnerability reporting:
[**Report a vulnerability**](https://github.com/cxyfer/urlet/security/advisories/new), also
reachable from the repository's *Security* tab. Only the maintainers can see the report.

A useful report includes:

- the urlet version or commit, and how it is deployed (Docker image, binary, reverse proxy in
  front of it, relevant `URLET_*` settings such as `URLET_TRUST_PROXY`);
- the affected endpoint, page or component;
- the steps to reproduce, ideally as `curl` commands or a short script;
- what an attacker gains, and which role they need (anonymous visitor, registered user, group
  member, administrator).

Test against an instance you run yourself. Do not test against instances you do not own, and do
not access or modify other people's data.

## What happens next

These are goals rather than guarantees (see the [disclaimer](#disclaimer)):

1. The report is acknowledged within 7 days.
2. The issue is confirmed or ruled out, and a severity is agreed with you; updates are posted in
   the advisory thread.
3. The fix is developed in a private fork of the advisory, then shipped in a patch release.
4. The GitHub Security Advisory is published with upgrade instructions once the release is
   available, crediting you unless you prefer to stay anonymous.

Please keep the details private until the advisory is published.

## Scope

In scope is the code in this repository: the `urlet` binary, its embedded assets and templates,
the container image built by [`Dockerfile`](Dockerfile), and the example deployment files under
[`deploy/`](deploy) and the `docker-compose*.yml` files.

The following are out of scope, or treated as hardening rather than vulnerabilities:

- **Actions that require administrator rights.** Administrators are trusted: they manage users,
  domains, settings and custom theme CSS. A way for a non-administrator to gain those rights is
  in scope.
- **Actions that require access to the host**, the SQLite database, the data directory or the
  secret key file.
- **Misconfigured deployments**, for example enabling `URLET_TRUST_PROXY` behind a proxy that
  forwards client-supplied `X-Real-IP` or `X-Forwarded-For` headers. Unclear or wrong deployment
  documentation is welcome as a regular issue.
- **Volumetric denial of service** and brute force that is only limited by the attacker's
  bandwidth. A way to bypass an existing limit, such as the login lockout, is in scope.
- **Short links pointing at malicious sites.** Redirecting to arbitrary URLs is what a URL
  shortener does; abuse handling is up to each operator. Bypassing a restriction the instance
  enforces, such as the guest target allowlist, is in scope.
- **Vulnerabilities in dependencies** without a demonstrated impact on urlet. Those belong
  upstream.
