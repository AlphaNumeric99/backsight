import { ShieldAlert } from "lucide-react";
import type { Camera } from "@/ipc";
import { strings } from "@/lib/strings";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useTrustCertificate } from "@/queries/cameras";

/** One fingerprint, broken into groups so two of them can be compared by eye. */
function Fingerprint({ label, value, tone }: { label: string; value?: string; tone: "muted" | "warning" }) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] font-medium uppercase tracking-[0.06em] text-fg-3">{label}</p>
      <p
        className={cn(
          "mt-1 break-all font-mono text-[12px] leading-[1.6]",
          tone === "warning" ? "text-warning" : "text-fg-2",
        )}
      >
        {value ? value.replace(/(.{4})/g, "$1 ").trim() : "—"}
      </p>
    </div>
  );
}

/**
 * Shown when a camera's pinned certificate no longer matches. Puts both fingerprints and the
 * likely reasons in front of the owner, because trusting the new one is their decision — the
 * app can't tell a firmware update from something answering in the camera's place.
 */
export function CertificateNotice({ camera }: { camera: Camera }) {
  const trust = useTrustCertificate();
  const { pinnedCertificate, actualCertificate } = camera.status;

  return (
    <section
      aria-labelledby={`cert-${camera.id}`}
      className="mb-5 overflow-hidden rounded-card border border-danger/30 bg-danger/[0.06]"
    >
      <div className="flex flex-wrap items-start gap-4 p-5">
        <span className="grid size-10 shrink-0 place-items-center rounded-full bg-danger/15 text-danger">
          <ShieldAlert className="size-5" strokeWidth={1.9} />
        </span>
        <div className="min-w-0 flex-1">
          <h2 id={`cert-${camera.id}`} className="text-[15px] font-semibold text-fg">
            {strings.certificate.title}
          </h2>
          <p className="mt-1 text-sm text-fg-2">{strings.certificate.body}</p>

          <div className="mt-4 grid gap-4 rounded-[10px] bg-sunken/60 p-4 sm:grid-cols-2">
            <Fingerprint label={strings.certificate.expected} value={pinnedCertificate} tone="muted" />
            <Fingerprint label={strings.certificate.actual} value={actualCertificate} tone="warning" />
          </div>

          <p className="mt-4 text-sm text-fg-2">{strings.certificate.benign}</p>
          <p className="mt-2 text-sm text-fg-2">{strings.certificate.risk}</p>

          <div className="mt-4 flex flex-wrap items-center gap-3">
            <Button variant="primary" onClick={() => trust.mutate(camera.id)} disabled={trust.isPending}>
              {trust.isPending ? strings.certificate.trusting : strings.certificate.trust}
            </Button>
            <p className="text-xs text-fg-3">{strings.certificate.verifyNote}</p>
          </div>
        </div>
      </div>
    </section>
  );
}
