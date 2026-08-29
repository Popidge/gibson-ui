const installCommand =
  "curl -fsSL https://raw.githubusercontent.com/Popidge/gibson-ui/main/install.sh | bash";

document.querySelectorAll("[data-copy]").forEach((copyButton) => {
  const copyLabel = copyButton.querySelector("[data-copy-label]");

  copyButton.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(installCommand);
      copyLabel.textContent = "Copied";
      window.setTimeout(() => {
        copyLabel.textContent = "Copy";
      }, 1800);
    } catch {
      const code = copyButton.closest(".install-line")?.querySelector("code");
      if (!code) return;
      const range = document.createRange();
      range.selectNodeContents(code);
      window.getSelection()?.removeAllRanges();
      window.getSelection()?.addRange(range);
      code.focus();
      copyLabel.textContent = "Selected";
    }
  });
});

const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
const heroVideo = document.querySelector(".hero video");

if (motionQuery.matches) {
  heroVideo?.pause();
}

document.addEventListener("visibilitychange", () => {
  if (!heroVideo || motionQuery.matches) return;
  if (document.hidden) heroVideo.pause();
  else heroVideo.play().catch(() => {});
});

const revealItems = document.querySelectorAll(".reveal");

if (motionQuery.matches || !("IntersectionObserver" in window)) {
  revealItems.forEach((item) => item.classList.add("is-visible"));
} else {
  const revealObserver = new IntersectionObserver(
    (entries, observer) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      });
    },
    { rootMargin: "0px 0px -10%", threshold: 0.08 },
  );

  revealItems.forEach((item) => revealObserver.observe(item));
}
