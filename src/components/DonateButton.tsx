import { DONATE_URL } from "../lib/web/config";
import { Heart } from "./Icons";

async function openExternal(url: string): Promise<void> {
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

export function DonateButton() {
  return (
    <button
      type="button"
      className="btn-ghost btn-sm donate-btn"
      aria-label="Donate"
      title="Support Mándalo on GitHub Sponsors"
      onClick={() => void openExternal(DONATE_URL)}
    >
      <Heart size={13} />
      <span>Donate</span>
    </button>
  );
}
