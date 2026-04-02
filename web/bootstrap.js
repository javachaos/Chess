const canvas = document.getElementById("chess-canvas");
const loadingCard = document.querySelector("[data-loading]");

function formatError(error) {
  if (error instanceof Error) {
    return {
      message: error.message || "The WebAssembly app failed to start.",
      details: error.stack || "",
    };
  }

  if (typeof error === "string") {
    return { message: error, details: "" };
  }

  const message = error && typeof error.toString === "function"
    ? error.toString()
    : "Unknown startup error";
  const details = error && typeof error === "object"
    ? JSON.stringify(error, null, 2)
    : "";

  return { message, details };
}

function setLoadingMessage(message) {
  const heading = loadingCard?.querySelector("h2");
  if (heading) {
    heading.textContent = message;
  }
}

function showError(message, details = "") {
  if (loadingCard) {
    const heading = loadingCard.querySelector("h2");
    const copy = loadingCard.querySelector(".overlay-copy");
    if (heading) {
      heading.textContent = "Board launch issue";
    }
    if (copy) {
      copy.textContent = message;
    }
  }
  console.error(message);
  if (details) {
    console.error(details);
  }
}

function watchForPanic() {
  if (window.app_has_panicked && window.app_has_panicked()) {
    showError(
      window.app_panic_message?.() || "The chess engine panicked after startup.",
      window.app_panic_callstack?.() || "",
    );
    return;
  }

  window.requestAnimationFrame(watchForPanic);
}

async function boot() {
  if (typeof WebAssembly === "undefined") {
    showError(
      "This browser or embedded preview does not support WebAssembly.",
      "Open the local site in Safari, Chrome, or Firefox instead of an in-app preview.",
    );
    return;
  }

  try {
    setLoadingMessage("Loading engine...");
    const {
      default: init,
      app_has_panicked,
      app_panic_callstack,
      app_panic_message,
      destroy_app,
      start_app,
    } = await import("./pkg/chess_engine.js");
    await init();
    window.app_has_panicked = app_has_panicked;
    window.app_panic_callstack = app_panic_callstack;
    window.app_panic_message = app_panic_message;
    window.destroy_app = destroy_app;

    setLoadingMessage("Starting board...");
    await start_app(canvas);

    document.documentElement.classList.add("engine-ready");
    if (loadingCard) {
      loadingCard.hidden = true;
    }
    watchForPanic();
  } catch (error) {
    const formatted = formatError(error);
    showError(formatted.message, formatted.details);
  }
}

window.addEventListener("pagehide", () => {
  window.destroy_app?.();
});

boot();
