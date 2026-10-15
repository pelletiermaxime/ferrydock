const choices = [...document.querySelectorAll('[data-feature]')];
const image = document.querySelector('#feature-image');
const screenshotLink = document.querySelector('#screenshot-link');
let selection = 0;

// Keep the previous frame until the selected screenshot has loaded.
async function selectFeature(choice) {
  const request = ++selection;
  const nextImage = new Image();
  nextImage.src = choice.href;
  try {
    await nextImage.decode();
  } catch {
    return;
  }
  if (request !== selection) return;
  image.src = choice.href;
  image.alt = choice.dataset.alt;
  screenshotLink.href = choice.href;
  screenshotLink.setAttribute('aria-label', `Open full-size screenshot: ${choice.textContent.trim()}`);
  document.querySelector('#feature-title').textContent = choice.dataset.title;
  document.querySelector('#feature-description').textContent = choice.dataset.description;
  choices.forEach(item => {
    const active = item === choice;
    item.classList.toggle('active', active);
    if (active) item.setAttribute('aria-current', 'true');
    else item.removeAttribute('aria-current');
  });
}

choices.forEach((choice, index) => {
  choice.addEventListener('click', event => {
    if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    selectFeature(choice);
  });
  choice.addEventListener('keydown', event => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? choices.length - 1
      : (index + (event.key === 'ArrowRight' ? 1 : -1) + choices.length) % choices.length;
    choices[next].focus();
    selectFeature(choices[next]);
  });
});

// Download URLs live in the HTML and follow GitHub’s latest-release redirect.
const platform = navigator.userAgentData?.platform || navigator.platform || '';
const mobile = /Android|iPhone|iPad|iPod/.test(navigator.userAgent) ||
  (/Mac/.test(platform) && navigator.maxTouchPoints > 1);
const linux = !mobile && /Linux/i.test(platform) && !/arm|aarch/i.test(platform);
const macos = !mobile && /Mac/i.test(platform);
const windows = !mobile && /Win/i.test(platform);
if (linux) {
  document.querySelector('[data-platform-download]').href = document.querySelector('[data-download="appimage"]').href;
  document.querySelector('[data-platform-label]').textContent = 'Download for Linux';
  document.querySelector('[data-platform-card="linux"]').classList.add('recommended');
} else if (macos) {
  // Browser platform strings cannot reliably distinguish Intel and Apple Silicon.
  document.querySelector('[data-platform-label]').textContent = 'Get FerryDock for macOS';
  document.querySelector('[data-platform-card="macos"]').classList.add('recommended');
} else if (windows) {
  // Show the x64 requirement and preview notes before choosing the ZIP.
  document.querySelector('[data-platform-label]').textContent = 'Get FerryDock for Windows';
  document.querySelector('[data-platform-card="windows"]').classList.add('recommended');
}
