import type { Metadata } from "next";
import { LegalPage } from "@/components/LegalPage";
import { REPO_URL } from "@/lib/site";

const title = "Privacy policy: Oeca";
const description =
  "What the Oeca app, its installer and this website do with your data. The app only connects to GitHub when you ask it to check for updates.";

export const metadata: Metadata = {
  title,
  description,
  alternates: { canonical: "/privacy/" },
  openGraph: {
    type: "website",
    siteName: "Oeca",
    title,
    description,
    url: "/privacy/",
    images: [
      { url: "/opengraph-image.png", width: 1200, height: 630, alt: "Oeca: a native notch for Windows" },
    ],
  },
  twitter: { card: "summary_large_image", title, description, images: ["/opengraph-image.png"] },
};

// Every statement about the app is checked against its source code: the
// network requests are the update check (src/update.rs, opt-in or by hand:
// settings_store.rs `check_for_updates`) and Oeca Hop (src/nearby.rs, off by
// default; its notice is `privacy_notice`), storage in clipboard_store.rs,
// settings_store.rs and nearby*.rs, Start on launch in startup.rs.
export default function Privacy() {
  return (
    <LegalPage
      title="Privacy policy"
      lead="Oeca is maintained by one person, Ujval Sai. This page explains what the Oeca app, its installer and this website do with information about you."
      updated={["2026-11-24", "24 November 2026"]}
    >
      <h2>The Oeca app</h2>
      <p>
        <strong>Oeca makes no network requests unless you turn on a feature that needs them</strong>: the update check and Oeca Hop, both off by default. It contains no
        telemetry, analytics, advertising or crash reporting, and it does not send anything about you or your PC to
        the developer. You can confirm this in the{" "}
        <a className="link" href={REPO_URL}>
          source code
        </a>
        . Oeca 0.2.0 and earlier make no network requests at all.
      </p>
      <p>
        From version 0.3.0, Oeca can check GitHub Releases for a newer version: when you choose Check for updates in
        Settings, under About, and once a day if you turn on Check for updates automatically there (it is off by
        default). Each check is one HTTPS request to GitHub&apos;s public API, <code>api.github.com</code>, with no
        account or personal data; it names Oeca and its version. GitHub receives the usual information about the
        request, such as your IP address. Oeca never downloads or installs anything itself: a newer version only
        shows a notice that opens its release page.
      </p>

      <h3>Oeca Hop</h3>
      <p>
        From version 0.5.0, Oeca Hop sends files between your PC and your phone or other devices. It is off until
        you turn it on in Settings, after a notice that explains the following, and it runs only while Oeca is open.
      </p>
      <ul>
        <li>
          While Hop is on, your PC connects to LocalSend&apos;s public signaling server{" "}
          (<code>public.localsend.org</code>) and STUN server (<code>stun.localsend.org</code>), run by the LocalSend
          project, not by Oeca. Both see your public IP address. The signaling server also sees your PC&apos;s Hop
          name, its device type and its Hop identity, and passes on connection requests, which carry that identity
          and your PC&apos;s network addresses.
        </li>
        <li>
          The signaling server lists your PC to every device that shares its public IP address: usually your own
          network, but on public Wi-Fi, a shared VPN or a mobile network that can include strangers. They can see
          that your PC is online, its Hop identity and its network addresses before you approve anything.
        </li>
        <li>
          Files go directly between the two devices, encrypted, never through those servers, and nothing is stored
          online. Nothing is received without your approval. Received files are saved in{" "}
          <code>Downloads\Oeca Hop</code>, marked as downloaded from the internet, and never opened by Hop.
        </li>
        <li>
          Your PC&apos;s Hop identity (protected with Windows data protection) and the devices you have verified
          are saved in <code>%LOCALAPPDATA%\Oeca</code>. Resetting the identity in Settings makes your devices
          verify it again.
        </li>
      </ul>
      <p>Everything below stays on your PC.</p>

      <h3>Clipboard history</h3>
      <ul>
        <li>
          While Oeca runs, it reads what you copy (text and images) to build the Clipboard space, keeping the last
          5, 10 or 20 items, as set in Settings. You can also add images by dragging files onto the notch.
        </li>
        <li>
          The history is saved to <code>%LOCALAPPDATA%\Oeca\clipboard-history.bin</code> so it survives Oeca
          restarting. This file is <strong>not encrypted</strong>, so anyone with access to your Windows account
          can read it.
        </li>
        <li>
          It belongs to your current Windows sign-in: a new session (after signing out or restarting Windows) does
          not show it and deletes the file.
        </li>
        <li>
          Deleting an item or choosing Clear removes it from the saved file straight away. Oeca never changes what
          is on the Windows clipboard itself.
        </li>
      </ul>

      <h3>Media information</h3>
      <p>
        To show what is playing, Oeca reads the title, artist, artwork and playback state that media apps share
        through the Windows media controls, and the name of the app that is playing. It displays them and does
        not save or send them anywhere.
      </p>

      <h3>Settings</h3>
      <p>
        Your preferences (such as accent colour, closing delay and clipboard size) are saved in{" "}
        <code>%LOCALAPPDATA%\Oeca\settings.ini</code>.
      </p>

      <h3>Windows registry</h3>
      <ul>
        <li>
          Oeca 0.4.0 and earlier: <code>HKEY_CURRENT_USER\Software\Oeca\Session</code>, a temporary key that
          identifies your current sign-in, so the clipboard history is never shown in a later session. Windows
          removes it when you sign out or restart. From 0.5.0, Oeca asks Windows for your sign-in instead and
          stores nothing in the registry for it.
        </li>
        <li>
          Only if you turn on <strong>Start on launch</strong>: an <code>Oeca</code> entry under{" "}
          <code>HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run</code>, so Oeca starts when you
          sign in. Turning the setting off, or uninstalling Oeca, removes it.
        </li>
      </ul>

      <h3>Removing your data</h3>
      <p>
        Uninstalling Oeca (Settings, Apps, Installed apps) removes the app, its shortcuts and its Start on launch
        entry, and keeps your settings and clipboard history in <code>%LOCALAPPDATA%\Oeca</code>. To remove them
        too, quit Oeca and delete that folder. Files received with Oeca Hop stay in <code>Downloads\Oeca Hop</code>.
      </p>

      <h2>The installer</h2>
      <p>
        The installer makes no network requests. It installs Oeca for your Windows account only, in{" "}
        <code>%LOCALAPPDATA%\Programs\Oeca</code>, and adds an entry to Installed apps so you can uninstall it.
      </p>

      <h2>Downloads from GitHub</h2>
      <p>
        Oeca is downloaded from GitHub Releases. GitHub receives the usual information about that request, such as
        your IP address, under the{" "}
        <a className="link" href="https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement">
          GitHub General Privacy Statement
        </a>
        . The same applies when you open an issue or view the source on GitHub.
      </p>

      <h2>This website</h2>
      <ul>
        <li>
          The site sets no cookies and uses no analytics, advertising or tracking scripts, and it has no forms or
          accounts.
        </li>
        <li>
          Its fonts are loaded from Fontshare (<code>api.fontshare.com</code> and <code>cdn.fontshare.com</code>),
          run by the Indian Type Foundry, so your browser sends them the usual request information, such as your IP
          address and browser details.
        </li>
        <li>
          Like any website, the service that hosts it receives standard request information (such as IP address,
          requested page and browser details) to deliver the pages. Its own privacy policy applies to that
          information.
        </li>
      </ul>

      <h2>Questions and changes</h2>
      <p>
        If you have a question about this policy, open an issue on{" "}
        <a className="link" href={`${REPO_URL}/issues`}>
          GitHub
        </a>
        . When this policy changes, the date at the top of this page changes, and the history of every change is
        public in the repository.
      </p>
    </LegalPage>
  );
}
