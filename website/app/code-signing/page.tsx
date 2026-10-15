import type { Metadata } from "next";
import { LegalPage } from "@/components/LegalPage";
import { RELEASES_URL, REPO_URL } from "@/lib/site";

const title = "Code signing policy: Oeca";
const description =
  "How Oeca releases are built and signed. Free code signing through SignPath Foundation is pending; current releases are not signed.";

export const metadata: Metadata = {
  title,
  description,
  alternates: { canonical: "/code-signing/" },
  openGraph: {
    type: "website",
    siteName: "Oeca",
    title,
    description,
    url: "/code-signing/",
    images: [
      { url: "/opengraph-image.png", width: 1200, height: 630, alt: "Oeca: a native notch for Windows" },
    ],
  },
  twitter: { card: "summary_large_image", title, description, images: ["/opengraph-image.png"] },
};

const WORKFLOW_URL = `${REPO_URL}/blob/master/.github/workflows/release.yml`;

// Written against the SignPath Foundation terms (https://signpath.org/terms).
// Keep the status honest: change it only once each step has really happened.
export default function CodeSigning() {
  return (
    <LegalPage
      title="Code signing policy"
      lead="How official Oeca releases are built, approved and signed, and who is responsible for each step."
      updated={["2026-10-15", "15 October 2026"]}
    >
      <h2>Status: pending</h2>
      <p>
        Oeca is applying for free code signing from SignPath Foundation. The application has not been
        accepted yet. <strong>No Oeca release is code-signed at the moment</strong>, including v0.2.0,
        so Windows SmartScreen may ask you to confirm before running the installer. This page will be updated when
        signing is active, and only signed files will be described as signed.
      </p>
      <p>Once the application is accepted, signed releases will carry this attribution:</p>
      <blockquote>
        Free code signing provided by{" "}
        <a className="link" href="https://signpath.io">
          SignPath.io
        </a>
        , certificate by{" "}
        <a className="link" href="https://signpath.org">
          SignPath Foundation
        </a>
      </blockquote>

      <h2>What is signed</h2>
      <p>
        Only Oeca&rsquo;s own release files, built from its{" "}
        <a className="link" href={REPO_URL}>
          public repository
        </a>
        : <code>Oeca.exe</code> and the installer <code>Oeca-Setup.exe</code>. Third-party binaries are not signed
        under Oeca&rsquo;s certificate. The installer is built with Inno Setup, whose own setup files are
        included in it unmodified.
      </p>

      <h2>How releases are built</h2>
      <ul>
        <li>
          Every release is built from source by the{" "}
          <a className="link" href={WORKFLOW_URL}>
            GitHub Actions workflow
          </a>{" "}
          in the repository, on a GitHub-hosted Windows runner: it checks formatting, runs Clippy and the tests,
          builds <code>Oeca.exe</code> with <code>cargo build --release --locked</code> and packages the installer
          with Inno Setup.
        </li>
        <li>
          Once signing is active, the workflow will submit those build outputs to SignPath, and the signed files
          will be published on{" "}
          <a className="link" href={RELEASES_URL}>
            GitHub Releases
          </a>
          . Files built anywhere else, including on the maintainer&rsquo;s PC, will not be signed.
        </li>
        <li>
          The build scripts and workflow are part of the source code and are reviewed as carefully as any other
          change.
        </li>
      </ul>

      <h2>Team roles</h2>
      <p>
        Oeca has one maintainer, so each role is currently held by the same person:{" "}
        <a className="link" href="https://github.com/ujvall">
          Ujval Sai (@ujvall)
        </a>
        .
      </p>
      <ul>
        <li>
          <strong>Committers and reviewers:</strong> Ujval Sai. Changes from anyone else, such as pull requests,
          are reviewed by the maintainer before they are merged.
        </li>
        <li>
          <strong>Approvers:</strong> Ujval Sai. Every signing request is approved manually in SignPath, one
          release at a time, after checking that it was built by the workflow above from a commit in this
          repository.
        </li>
      </ul>
      <p>
        The maintainer uses multi-factor authentication for GitHub and will use it for SignPath. If more people
        join the project, this page will list them and their roles.
      </p>

      <h2>Privacy</h2>
      <p>
        This program will not transfer any information to other networked systems unless specifically requested
        by the user or the person installing or operating it. See the{" "}
        <a className="link" href="/privacy/">
          Privacy policy
        </a>{" "}
        for what Oeca stores on your PC and what this website and GitHub receive.
      </p>

      <h2>More information</h2>
      <ul>
        <li>
          <a className="link" href="https://signpath.org/terms">
            SignPath Foundation terms for open-source projects
          </a>
        </li>
        <li>
          <a className="link" href="https://docs.signpath.io/trusted-build-systems/github">
            SignPath documentation: signing from GitHub Actions
          </a>
        </li>
        <li>
          Questions or concerns about a signed Oeca file: open an issue on{" "}
          <a className="link" href={`${REPO_URL}/issues`}>
            GitHub
          </a>
          .
        </li>
      </ul>
    </LegalPage>
  );
}
