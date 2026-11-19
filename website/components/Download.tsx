import Image from "next/image";
import { ArrowUpRight } from "@phosphor-icons/react/dist/ssr";
import { WindowsMark } from "./WindowsMark";
import { DOWNLOAD_URL, LICENSE_URL, RELEASES_URL, REQUIREMENTS, VERSION } from "@/lib/site";

export function Download() {
  return (
    <section id="download" className="section" aria-labelledby="download-title">
      <div className="shell">
        {/* Closes the page the way it opened: the same faint lit edge */}
        <div
          data-reveal
          className="panel relative isolate flex flex-col items-center overflow-hidden px-6 py-16 text-center md:py-24"
        >
          <div aria-hidden className="edge-light -z-10" />
          <Image
            src="/media/oeca-icon-256.webp"
            alt=""
            width={64}
            height={64}
            className="size-16 drop-shadow-[0_10px_24px_rgb(161_169_255/0.15)]"
          />
          <h2 id="download-title" className="t-title mt-8">
            Get Oeca for Windows
          </h2>
          <p className="t-lead mt-5 max-w-[32rem]">
            Free and open source under the{" "}
            <a className="link" href={LICENSE_URL}>
              MIT License
            </a>
            . Installs for your account, with no administrator rights.
          </p>
          <div className="mt-10 flex w-full flex-col justify-center gap-3 sm:w-auto sm:flex-row">
            <a className="btn btn-primary" href={DOWNLOAD_URL}>
              <WindowsMark size={15} />
              Download {VERSION}
            </a>
            <a className="btn btn-ghost group" href={RELEASES_URL}>
              All releases
              <ArrowUpRight
                size={16}
                aria-hidden
                className="text-muted transition-[transform,color] duration-200 ease-out group-hover:-translate-y-0.5 group-hover:translate-x-0.5 group-hover:text-paper"
              />
            </a>
          </div>
          <p className="t-caption mt-6 max-w-md">
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
