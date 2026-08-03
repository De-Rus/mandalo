(function () {
  const tabs = Array.from(document.querySelectorAll("nav button"));
  const panels = Array.from(document.querySelectorAll("section[data-panel]"));

  function select(name) {
    for (const tab of tabs) tab.setAttribute("aria-selected", String(tab.dataset.tab === name));
    for (const panel of panels) panel.hidden = panel.dataset.panel !== name;
  }

  for (const tab of tabs) {
    tab.addEventListener("click", () => select(tab.dataset.tab));
  }

  if (tabs.length > 0) select(tabs[0].dataset.tab);
})();
