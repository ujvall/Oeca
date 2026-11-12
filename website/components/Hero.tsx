import { ArrowRight, ClipboardText, Clock, MusicNotes, Timer } from "@phosphor-icons/react/dist/ssr";
import { WindowsMark } from "./WindowsMark";
import { DOWNLOAD_URL, REPO_URL, VERSION } from "@/lib/site";

// What lives in the notch, drawn as icons: an emblem, not a capture of the app.
const NOTCH = [Clock, MusicNotes, Timer, ClipboardText];

export function Hero() {
  return (
    <section id="top" className="pt-16">
      <div className="shell">
        {/* A notch made of icons, hanging from the top edge like the real one */}
        <div
          aria-hidden
          data-intro
          className="mx-auto flex w-fit items-center gap-1 rounded-b-[1.375rem] bg-[#060607] px-2.5 pt-2 pb-2.5 shadow-[inset_0_-1px_0_rgb(255_255_255/0.08),0_20px_40px_-20px_rgb(0_0_0/0.8)]"
        >
          {NOTCH.map((Icon, i) => (
            <span
              key={i}
              className="grid size-9 place-items-center rounded-full text-muted transition-colors duration-200 hover:bg-white/[0.06] hover:text-accent"
            >
              <Icon size={18} weight="duotone" />
            </span>
          ))}
        </div>

        <div className="pt-20 pb-12 md:pt-28 md:pb-16">
          <h1 data-intro className="t-display max-w-[13ch]">
            The top of your screen, put to work
          </h1>
          <p data-intro className="t-lead mt-6 max-w-[34rem]">
            Oeca is a native notch for Windows. The time, what&rsquo;s playing, a countdown and your clipboard,
            one click away.
          </p>
          <div data-intro className="mt-10 flex flex-col gap-3 sm:flex-row">
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
      </div>
    </section>
  );
}
