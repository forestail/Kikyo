const ABOUT_CONTRIBUTORS_HEADING = "配列検証にご協力いただいた皆さま";
const ABOUT_CONTRIBUTORS_NOTE = "（敬称略・順不同）";
const DEFAULT_INLINE_VISIBLE_COUNT = 12;

// Keep all contributor handles in one place.
// Add names here when available.
export const ABOUT_ARRAY_VALIDATION_CONTRIBUTORS = [
  // "HN1",
  // "HN2",
];

function normalizeContributorNames(contributors) {
  if (!Array.isArray(contributors)) return [];
  return contributors
    .map((name) => (typeof name === "string" ? name.trim() : ""))
    .filter((name) => name.length > 0);
}

function createContributorViewModel(allContributors, inlineVisibleCount) {
  const visible = allContributors.slice(0, inlineVisibleCount);
  const remainingCount = Math.max(0, allContributors.length - visible.length);
  return {
    all: allContributors,
    visible,
    remainingCount,
    hasOverflow: remainingCount > 0,
  };
}

function createContributorsList(names) {
  const list = document.createElement("ul");
  list.className = "about-contributors-list";
  for (const name of names) {
    const item = document.createElement("li");
    item.className = "about-contributors-item";
    item.textContent = name;
    list.appendChild(item);
  }
  return list;
}

export function mountAboutContributors(
  root,
  {
    contributors = ABOUT_ARRAY_VALIDATION_CONTRIBUTORS,
    inlineVisibleCount = DEFAULT_INLINE_VISIBLE_COUNT,
  } = {},
) {
  if (!(root instanceof HTMLElement)) return;

  const allContributors = normalizeContributorNames(contributors);
  root.replaceChildren();

  if (allContributors.length === 0) {
    root.hidden = true;
    return;
  }

  const viewModel = createContributorViewModel(allContributors, inlineVisibleCount);

  // For now we always render all names.
  // This view model keeps the "他◯名 + すべて表示" extension straightforward.
  const namesToRender = viewModel.all;

  root.hidden = false;
  root.dataset.hasOverflow = String(viewModel.hasOverflow);
  root.dataset.remainingCount = String(viewModel.remainingCount);

  const section = document.createElement("section");
  section.className = "about-contributors";

  const heading = document.createElement("p");
  heading.className = "about-contributors-heading";
  heading.textContent = ABOUT_CONTRIBUTORS_HEADING;
  section.appendChild(heading);

  const note = document.createElement("p");
  note.className = "about-contributors-note";
  note.textContent = ABOUT_CONTRIBUTORS_NOTE;
  section.appendChild(note);

  section.appendChild(createContributorsList(namesToRender));
  root.appendChild(section);
}
