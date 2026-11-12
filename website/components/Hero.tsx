import { DOWNLOAD_URL, REPO_URL, VERSION } from "@/lib/site";

export function Hero() {
  return (
    <section id="top" className="pt-36 pb-12 md:pt-44 md:pb-16">
      <div className="shell">
        <h1 data-intro className="t-display max-w-[13ch]">
          The top of your screen, put to work
        </h1>
        <p data-intro className="t-lead mt-6 max-w-[34rem]">
          Oeca is a native notch for Windows. The time, what&rsquo;s playing, a countdown and your clipboard,
          one click away.
        </p>
        <div data-intro className="mt-10 flex flex-col gap-3 sm:flex-row">
          <a className="btn btn-primary" href={DOWNLOAD_URL}>
            Download for Windows
          </a>
          <a className="btn btn-ghost" href={REPO_URL}>
            View on GitHub
          </a>
        </div>
        <p data-intro className="t-caption mt-5">
          Version {VERSION}. Free and open source.
        </p>
      </div>
    </section>
  );
}
