import type { Metadata, Viewport } from "next";
import { SITE_URL } from "@/lib/site";
import "./globals.css";

const description =
  "A native notch for the top of your Windows desktop: the time, what's playing, a timer and your clipboard history, one click away.";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: "Oeca: a native notch for Windows",
  description,
  applicationName: "Oeca",
  icons: {
    icon: [
      { url: "/favicon.ico", sizes: "any" },
      { url: "/media/oeca-icon-32.png", type: "image/png", sizes: "32x32" },
      { url: "/media/oeca-icon-64.png", type: "image/png", sizes: "64x64" },
    ],
    apple: "/media/oeca-icon-180.png",
  },
  openGraph: {
    type: "website",
    title: "Oeca: a native notch for Windows",
    description,
    siteName: "Oeca",
  },
  twitter: {
    card: "summary_large_image",
    title: "Oeca: a native notch for Windows",
    description,
  },
};

export const viewport: Viewport = {
  themeColor: "#0b0b0b",
  colorScheme: "dark",
};

// Runs before first paint: hides intro elements only when motion is allowed,
// and un-hides them after 2.5 s if the animation never starts.
const motionGate = `(function(){var d=document.documentElement;if(!matchMedia('(prefers-reduced-motion: reduce)').matches){d.classList.add('motion');setTimeout(function(){if(!window.__oecaIntro)d.classList.remove('motion')},2500)}})();`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: motionGate }} />
        {/* General Sans, served by Fontshare under the ITF Free Font License */}
        <link rel="preconnect" href="https://api.fontshare.com" crossOrigin="" />
        <link rel="preconnect" href="https://cdn.fontshare.com" crossOrigin="" />
        <link
          rel="stylesheet"
          href="https://api.fontshare.com/v2/css?f[]=general-sans@400,500,600&display=swap"
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
