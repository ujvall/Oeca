import { ArrowRight } from "@phosphor-icons/react/dist/ssr";
import { WindowsMark } from "./WindowsMark";
import { DOWNLOAD_URL, REPO_URL, VERSION } from "@/lib/site";

export function Hero() {
  return (
    <section id="top">
      <div className="shell pt-32 pb-14 md:pt-40 md:pb-20">
        <p data-intro className="t-mono text-muted">
          Version {VERSION} for Windows 10 and 11
        </p>
        <h1 data-intro className="t-display mt-5 max-w-[16ch]">
          The top of your screen, put to work
        </h1>
        <p data-intro className="t-lead mt-5 max-w-[34rem]">
          Oeca is a native notch for Windows. The time, what&rsquo;s playing, a countdown and your clipboard, one
          click away.
        </p>
        <div data-intro className="mt-8 flex flex-col gap-3 sm:flex-row">
          <a className="btn btn-primary" href={DOWNLOAD_URL}>
            <WindowsMark size={14} />
            Download for Windows
          </a>
          <a className="btn btn-ghost group" href={REPO_URL}>
            View on GitHub
            <ArrowRight
              size={16}
              aria-hidden
              className="text-muted transition-[transform,color] duration-200 ease-out group-hover:translate-x-0.5 group-hover:text-paper"
            />
          </a>
        </div>
      </div>
    </section>
  );
}
