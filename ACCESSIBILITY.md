# Accessibility

Moss should be usable by everyone who runs a game server, including people who use a keyboard only, a screen reader, a magnifier, high contrast or reduced motion. This file says what we aim for, what is in place, how it is tested, what is known to fall short, and how to tell us when something does not work for you.

## Our target

The web panel (and the desktop app, which shows the same panel) aims for **[WCAG 2.2](https://www.w3.org/TR/WCAG22/) level AA**. We have not had an external audit, so this is a goal we test against, not a certified claim.

## Accessibility settings

Open **Settings → Accessibility**. The choices are saved in your browser, like the appearance settings, and apply straight away.

| Setting | Options | What it does |
|---|---|---|
| Text size | 100%, 115%, 130%, 150% | Scales text across the panel; buttons and fields grow with it. Browser zoom works too, on top of this. |
| Motion | Follow system, Reduce | Reduce turns off animations and transitions. Moss also follows the system's reduced-motion setting. |
| Focus indicator | Standard, Strong | Strong draws a thick ring around whatever has keyboard focus, text fields included. |
| Underline links | On hover, Always | Links in text are always underlined, so they don't rely on colour alone. |
| Text spacing | Standard, Wider | Adds line, letter and word spacing for easier reading. |
| Transparency and effects | Standard, Reduce | Removes the background grain, glows and see-through overlays. |
| System high contrast | Follow, Ignore | With the System theme, Moss switches to High contrast when your device asks for more contrast. |

**Reset accessibility settings** puts them all back to the defaults.

Under **Settings → Appearance**:
- The **High contrast** theme is available, along with thirteen other themes. Each one's text, accent and status colours are tested against WCAG AA (4.5:1).
- Density can be **Compact** or **Comfortable**.
- You can change the colours of the Deep forest and Paper themes (and System, which uses them) with the **theme editor**. It checks every text and background pair against WCAG AA (4.5:1) and will not apply colours that fail.

## What is in place

- **Keyboard:**
  - The panel is built to be used with the keyboard alone.
  - A *Skip to content* link comes first.
  - Dialogs keep focus inside and return it when they close.
  - <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> opens the command palette, which can reach any page, server, backup, content item or file. There are no single-key shortcuts that could fire by accident.
- **Screen readers:**
  - Pages use landmarks and headings.
  - Controls have names.
  - Option groups are radio groups.
  - Charts have a text summary.
  - Status changes such as operation progress and validation errors are announced through live regions.
- **Colour:**
  - Light, dark, high-contrast and eleven other themes are available.
  - Server and operation states have text labels on server pages, lists and badges.
- **Layout:** the panel works at phone width without horizontal scrolling, and at 150% text size.

## How it is tested

- Every page is checked with [axe-core](https://github.com/dequelabs/axe-core) against WCAG 2 A and AA, in the light and dark themes, on every pull request (`npm run test:ui` in `web/`, run in CI). Serious or critical findings fail the build.
- Every theme's colour pairs are checked for 4.5:1 contrast by a unit test, and each added theme is run through axe on the main pages.
- The accessibility settings have their own test. It turns every option on, checks that each one takes effect and survives a reload, runs axe with all of them on, checks that the System theme follows the device's contrast request, and checks that pages don't scroll sideways at phone width with 150% text and wider spacing.
- Visual snapshots on desktop and phone catch layout regressions (`npm run test:visual`, also run in CI).
- Automated tools catch only part of what matters, so new and changed screens should also be tried with the keyboard alone (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Known limitations

- The server console and the file editor show many lines of server output. Screen readers can read them line by line, but new console output is not announced as it arrives, because a busy server would flood the reader. Use the console's search and the level filters instead.
- In the sidebar, a server's state is shown as a coloured dot to save space. Screen readers get it as text; everyone else can see it in words on the server list and the server's own page.
- Some charts show data as shapes. Their text summary gives the key numbers, not every data point.
- Game servers, mods and plugins have their own interfaces, such as in-game menus, map plugins and mod pages. Moss can't change those.
- Moss has not been tested with every combination of screen reader and browser. Reports from people who use them are very welcome.

## Reporting a problem

If something in Moss is hard or impossible for you to use, please [open an accessibility issue](https://github.com/T1nkiePlayz/Moss/issues/new?template=accessibility.yml). It helps to tell us:
- What you were trying to do.
- What happened.
- Any assistive technology, browser and operating system you use.

You don't need to know which guideline is involved. An accessibility problem is a bug.

The project website is a separate repository, [T1nkiePlayz/Moss-Website](https://github.com/T1nkiePlayz/Moss-Website). Problems with the site can be reported there.
