# General Sans (not included in this repository)

Official Oeca releases embed three static weights of General Sans, by the
Indian Type Foundry, so the app looks the same on every PC:

- `GeneralSans-Regular.otf` (400)
- `GeneralSans-Medium.otf` (500)
- `GeneralSans-Semibold.otf` (600)

General Sans is not open source. It is licensed under the
[ITF Free Font License](https://www.fontshare.com/licenses/itf-ffl), which
allows embedding it in your own applications but does not allow publishing
the font files in a public repository. They are therefore git-ignored.

To build with the font, download General Sans from
[Fontshare](https://www.fontshare.com/fonts/general-sans) yourself and copy
the three files above, unmodified, from `Fonts/OTF/` into this folder. Without
them Oeca still builds and runs: it uses General Sans if it is installed on
the PC, otherwise Satoshi, Bahnschrift or Segoe UI Variable.
