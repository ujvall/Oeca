import { DOWNLOAD_URL, LICENSE_URL, RELEASES_URL, REQUIREMENTS, VERSION } from "@/lib/site";

export function Download() {
  return (
    <section id="download" className="section" aria-labelledby="download-title">
      <div className="shell">
        <div data-reveal className="max-w-2xl">
          <h2 id="download-title" className="t-title">
            Get Oeca for Windows
          </h2>
          <p className="t-lead mt-5">
            Free and open source under the{" "}
            <a className="link" href={LICENSE_URL}>
              MIT License
            </a>
            . Installs for your account, with no administrator rights.
          </p>
          <div className="mt-10 flex flex-col gap-3 sm:flex-row">
            <a className="btn btn-primary" href={DOWNLOAD_URL}>
              Download {VERSION}
            </a>
            <a className="btn btn-ghost" href={RELEASES_URL}>
              All releases
            </a>
          </div>
          <p className="t-caption mt-5 max-w-lg">
            {REQUIREMENTS}. Not code-signed yet, so SmartScreen may ask you to confirm. See the{" "}
            <a className="link" href="/code-signing/">
              code signing policy
            </a>
            .
          </p>
        </div>
      </div>
    </section>
  );
}
