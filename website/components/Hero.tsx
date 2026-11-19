import { ArrowRight } from "@phosphor-icons/react/dist/ssr";
import { WindowsMark } from "./WindowsMark";
import { DOWNLOAD_URL, REPO_URL, VERSION } from "@/lib/site";

export function Hero() {
  return (
    <section id="top" className="relative isolate overflow-hidden">
      {/* The faintly lit top edge the notch (Nav) hangs from */}
      <div aria-hidden className="edge-light -z-10" />

      <div className="shell flex flex-col items-center pt-40 pb-24 text-center md:pt-52 md:pb-32">
        <h1 data-intro className="t-display max-w-[12ch]">
          The top of your screen, put to work
        </h1>
        <p data-intro className="t-lead mt-7 max-w-[34rem]">
          Oeca is a native notch for Windows. The time, what&rsquo;s playing, a countdown and your clipboard, one
          click away.
        </p>
        <div data-intro className="mt-10 flex w-full flex-col justify-center gap-3 sm:w-auto sm:flex-row">
          <a className="btn btn-primary" href={DOWNLOAD_URL}>
            <WindowsMark size={15} />
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
        <p data-intro className="t-caption mt-5">
          Version {VERSION}. Free and open source.
        </p>
      </div>
    </section>
  );
}
