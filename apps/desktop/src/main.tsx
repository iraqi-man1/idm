import React from "react";
import ReactDOM from "react-dom/client";
import { Toaster } from "sonner";
import App from "@/App";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import "@/i18n";
import "@/styles/globals.css";
import { CaptureWindow } from "@/windows/CaptureWindow";
import { ProgressWindow } from "@/windows/ProgressWindow";

// Secondary windows share the bundle and are selected by the URL hash.
function Root() {
  const hash = window.location.hash;
  const progress = hash.match(/^#\/progress\/([0-9a-f-]{36})$/i);
  if (progress) {
    return (
      <>
        <ProgressWindow id={progress[1]} />
        <Toaster position="bottom-center" richColors />
      </>
    );
  }
  const capture = hash.match(/^#\/capture\/([0-9a-f]{16})$/i);
  if (capture) {
    return (
      <>
        <CaptureWindow id={capture[1]} />
        <Toaster position="bottom-center" richColors />
      </>
    );
  }
  return <App />;
}

// Block the browser context menu outside our own menus and text fields.
window.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement;
  if (!el.closest("input, textarea, [data-selectable]")) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <Root />
    </ErrorBoundary>
  </React.StrictMode>,
);
