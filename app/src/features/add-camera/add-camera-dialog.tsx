import { useEffect, useId, useState, type FormEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { AnimatePresence, motion } from "motion/react";
import {
  BellRing,
  Cctv,
  ChevronRight,
  CircleAlert,
  CircleHelp,
  Info,
  KeyRound,
  RotateCw,
  Router,
  type LucideIcon,
} from "lucide-react";
import { api, type ApiError, type Camera, type DeviceKind, type DiscoveredDevice } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { describeError, toApiError } from "@/lib/errors";
import { minutesUntil, parseIso } from "@/lib/time";
import { Dialog, DialogBody, DialogFooter, DialogForm } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Field, Input, PasswordInput } from "@/components/ui/input";
import { Badge, Chip } from "@/components/ui/misc";
import { Disclosure } from "@/components/ui/disclosure";
import { useNow } from "@/hooks/use-now";
import { useAddCamera, useGroups } from "@/queries/cameras";
import { useUiStore } from "@/state/ui";

type Step = "find" | "credentials" | "done";

interface Target {
  host: string;
  name?: string;
  model?: string;
  kind?: DeviceKind;
}

const KIND_ICON: Record<DeviceKind, LucideIcon> = {
  camera: Cctv,
  doorbell: BellRing,
  hub: Router,
  other: CircleHelp,
};

const IPV4 = /^(25[0-5]|2[0-4]\d|1?\d?\d)(\.(25[0-5]|2[0-4]\d|1?\d?\d)){3}$/;
const HOSTNAME = /^(?=.{1,253}$)[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/i;

export function isValidHost(value: string): boolean {
  const v = value.trim();
  return IPV4.test(v) || (HOSTNAME.test(v) && !/^\d+(\.\d+)*$/.test(v));
}

export function AddCameraDialog() {
  const open = useUiStore((s) => s.addCameraOpen);
  const setOpen = useUiStore((s) => s.setAddCameraOpen);
  const navigate = useNavigate();

  const [step, setStep] = useState<Step>("find");
  const [target, setTarget] = useState<Target | null>(null);
  const [added, setAdded] = useState<Camera | null>(null);
  const [scanId, setScanId] = useState(0);
  const add = useAddCamera();

  useEffect(() => {
    if (open) {
      setStep("find");
      setTarget(null);
      setAdded(null);
      setScanId((n) => n + 1);
      add.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reset only when the dialog opens
  }, [open]);

  const busy = add.isPending;
  const title =
    step === "find"
      ? strings.addCamera.title
      : step === "credentials"
        ? strings.addCamera.credentialsTitle
        : strings.addCamera.successTitle(added?.name ?? "");
  const description =
    step === "find"
      ? strings.addCamera.findDescription
      : step === "credentials"
        ? strings.addCamera.credentialsDescription
        : undefined;

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !busy && setOpen(o)}
      title={title}
      description={description}
      hideHeader={step === "done"}
      dismissible={!busy}
      size="md"
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={step}
          initial={{ opacity: 0, x: step === "find" ? -14 : 14 }}
          animate={{ opacity: 1, x: 0, transition: transitions.base }}
          exit={{ opacity: 0, x: step === "find" ? -14 : 14, transition: transitions.fast }}
          className="flex min-h-0 flex-1 flex-col"
        >
          {step === "find" && (
            <FindStep
              scanId={scanId}
              enabled={open}
              onRescan={() => setScanId((n) => n + 1)}
              onPick={(t) => {
                setTarget(t);
                add.reset();
                setStep("credentials");
              }}
            />
          )}
          {step === "credentials" && target && (
            <CredentialsStep
              target={target}
              busy={busy}
              error={add.error ? toApiError(add.error) : null}
              onBack={() => setStep("find")}
              onSubmit={(req) =>
                add.mutate(req, {
                  onSuccess: (camera) => {
                    setAdded(camera);
                    setStep("done");
                  },
                })
              }
            />
          )}
          {step === "done" && added && (
            <DoneStep
              camera={added}
              onClose={() => setOpen(false)}
              onOpenLive={() => {
                setOpen(false);
                void navigate({ to: "/cameras/$cameraId", params: { cameraId: added.id }, search: { tab: "live" } });
              }}
            />
          )}
        </motion.div>
      </AnimatePresence>
    </Dialog>
  );
}

// --- Step 1: find -----------------------------------------------------------------------------

function FindStep({
  scanId,
  enabled,
  onRescan,
  onPick,
}: {
  scanId: number;
  enabled: boolean;
  onRescan: () => void;
  onPick: (target: Target) => void;
}) {
  const scan = useQuery({
    queryKey: ["discover", scanId],
    queryFn: () => api.discover(3000),
    enabled,
    staleTime: Infinity,
    gcTime: 0,
    retry: false,
  });
  const [manual, setManual] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const devices = scan.data ?? [];
  const scanning = scan.isFetching;

  const submitManual = (e: FormEvent) => {
    e.preventDefault();
    if (!isValidHost(manual)) {
      setManualError(strings.addCamera.manualInvalid);
      return;
    }
    const host = manual.trim();
    const known = devices.find((d) => d.host === host);
    onPick({ host, name: known?.name, model: known?.model, kind: known?.kind });
  };

  return (
    <>
      <DialogBody className="pt-3">
        <div className="flex items-center gap-3.5 rounded-2xl bg-surface-2 p-3.5">
          <Radar active={scanning} />
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium text-fg" aria-live="polite">
              {scanning ? strings.addCamera.scanning : strings.addCamera.found(devices.length)}
            </p>
            {scanning && <p className="text-xs text-fg-3">{strings.addCamera.scanningHint}</p>}
          </div>
          <Button variant="ghost" size="sm" onClick={onRescan} disabled={scanning}>
            <RotateCw className={cn(scanning && "animate-spin")} />
            {strings.addCamera.rescan}
          </Button>
        </div>

        <div className="mt-2 max-h-[292px] min-h-[140px] overflow-y-auto">
          {scanning ? (
            <ul className="grid gap-1 py-1" aria-hidden>
              {[0, 1, 2].map((i) => (
                <li key={i} className="flex items-center gap-3 px-3 py-2.5">
                  <div className="skeleton size-10 rounded-xl" />
                  <div className="grid flex-1 gap-1.5">
                    <div className="skeleton h-3.5 w-1/3 rounded" />
                    <div className="skeleton h-3 w-1/2 rounded" />
                  </div>
                </li>
              ))}
            </ul>
          ) : devices.length === 0 ? (
            <div className="px-3 py-8 text-center">
              <p className="text-sm font-medium text-fg">{strings.addCamera.noneFound}</p>
              <p className="mx-auto mt-1 max-w-xs text-[13px] text-fg-2">{strings.addCamera.noneFoundBody}</p>
            </div>
          ) : (
            <ul className="grid gap-0.5 py-1">
              {devices.map((d, i) => (
                <motion.li
                  key={d.host}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0, transition: { ...transitions.base, delay: Math.min(i, 8) * 0.035 } }}
                >
                  <DeviceRow device={d} onPick={onPick} />
                </motion.li>
              ))}
            </ul>
          )}
        </div>
      </DialogBody>

      <form onSubmit={submitManual} noValidate className="shrink-0 border-t border-border px-6 pb-6 pt-4">
        <Field label={strings.addCamera.manualLabel} error={manualError}>
          {({ id, describedBy, invalid }) => (
            <div className="flex gap-2">
              <Input
                id={id}
                value={manual}
                onChange={(e) => {
                  setManual(e.target.value);
                  setManualError(null);
                }}
                placeholder={strings.addCamera.manualPlaceholder}
                inputMode="url"
                autoComplete="off"
                spellCheck={false}
                aria-describedby={describedBy}
                aria-invalid={invalid || undefined}
                className="font-mono"
              />
              <Button type="submit" variant="primary" className="h-10" disabled={!manual.trim()}>
                {strings.common.continue}
              </Button>
            </div>
          )}
        </Field>
      </form>
    </>
  );
}

function Radar({ active }: { active: boolean }) {
  return (
    <div className="relative grid size-11 shrink-0 place-items-center">
      {active &&
        [0, 0.8, 1.6].map((delay) => (
          <span
            key={delay}
            aria-hidden
            className="absolute inset-0 animate-radar rounded-full border-2 border-brand"
            style={{ animationDelay: `${delay}s` }}
          />
        ))}
      <span
        className={cn(
          "relative grid size-9 place-items-center rounded-full transition-colors duration-(--dur-base)",
          active ? "bg-brand text-white" : "bg-brand-soft text-brand-text",
        )}
      >
        <Cctv className="size-[18px]" strokeWidth={2} />
      </span>
    </div>
  );
}

function DeviceRow({ device, onPick }: { device: DiscoveredDevice; onPick: (t: Target) => void }) {
  const Icon = KIND_ICON[device.kind];
  const supported = device.kind === "camera" || device.kind === "doorbell";
  const disabled = device.alreadyAdded || !supported;
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={() => onPick({ host: device.host, name: device.name, model: device.model, kind: device.kind })}
      className={cn(
        "group flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors duration-(--dur-fast)",
        disabled ? "cursor-default" : "hover:bg-hover",
      )}
    >
      <span
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-xl",
          disabled ? "bg-surface-3 text-fg-3" : "bg-brand-soft text-brand-text",
        )}
      >
        <Icon className="size-5" strokeWidth={1.8} />
      </span>
      <span className="min-w-0 flex-1">
        <span className={cn("block truncate text-sm font-medium", disabled ? "text-fg-2" : "text-fg")}>
          {device.name ?? strings.addCamera.unnamed}
        </span>
        <span className="block truncate font-mono text-xs text-fg-3">
          {[device.model, device.host].filter(Boolean).join(" · ")}
        </span>
      </span>
      {device.alreadyAdded ? (
        <Badge tone="neutral" size="sm">
          {strings.addCamera.alreadyAdded}
        </Badge>
      ) : !supported ? (
        <Badge tone="outline" size="sm">
          {strings.addCamera.notSupported}
        </Badge>
      ) : (
        <ChevronRight className="size-4 text-fg-3 transition-transform group-hover:translate-x-0.5" />
      )}
    </button>
  );
}

// --- Step 2: credentials ----------------------------------------------------------------------

function CredentialsStep({
  target,
  busy,
  error,
  onBack,
  onSubmit,
}: {
  target: Target;
  busy: boolean;
  error: ApiError | null;
  onBack: () => void;
  onSubmit: (req: Parameters<typeof api.addCamera>[0]) => void;
}) {
  const groups = useGroups();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [accountOpen, setAccountOpen] = useState(false);
  const [username, setUsername] = useState("");
  const [accountPassword, setAccountPassword] = useState("");
  const [groupIds, setGroupIds] = useState<string[]>([]);
  const [compatOpen, setCompatOpen] = useState(false);
  const now = useNow(1000);
  const formId = useId();

  const lockedMins = error?.code === "camera_locked" && error.retryAt ? minutesUntil(error.retryAt, now) : 0;
  const lockedActive = error?.code === "camera_locked" && error.retryAt ? parseIso(error.retryAt) > now : false;
  const Icon = KIND_ICON[target.kind ?? "camera"];

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!password) {
      setPasswordError(strings.addCamera.passwordRequired);
      return;
    }
    onSubmit({
      host: target.host,
      name: name.trim() || undefined,
      cloudPassword: password,
      cameraAccount: accountOpen && username.trim() ? { username: username.trim(), password: accountPassword } : undefined,
      groupIds,
    });
  };

  return (
    <DialogForm id={formId} onSubmit={submit} noValidate>
      <DialogBody className="grid gap-4 pt-3">
        <div className="flex items-center gap-3 rounded-2xl bg-surface-2 p-3">
          <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-brand-soft text-brand-text">
            <Icon className="size-5" strokeWidth={1.8} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium text-fg">{target.name ?? strings.addCamera.unnamed}</span>
            <span className="block truncate font-mono text-xs text-fg-3">
              {[target.model, target.host].filter(Boolean).join(" · ")}
            </span>
          </span>
          <Button variant="ghost" size="sm" onClick={onBack} disabled={busy}>
            {strings.addCamera.change}
          </Button>
        </div>

        <AnimatePresence initial={false}>
          {error && (
            <motion.div
              key={`${error.code}-${error.retryAt ?? ""}`}
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto", transition: transitions.base }}
              exit={{ opacity: 0, height: 0, transition: transitions.fast }}
              className="overflow-hidden"
            >
              <ErrorBanner error={error} now={now} />
            </motion.div>
          )}
        </AnimatePresence>

        <Field label={strings.addCamera.nameLabel} optional>
          {({ id }) => (
            <Input
              id={id}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={target.name ?? strings.addCamera.namePlaceholder}
              autoComplete="off"
            />
          )}
        </Field>

        <Field label={strings.addCamera.passwordLabel} help={strings.addCamera.passwordHelp} error={passwordError}>
          {({ id, describedBy, invalid }) => (
            <PasswordInput
              id={id}
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setPasswordError(null);
              }}
              autoFocus
              required
              autoComplete="current-password"
              aria-describedby={describedBy}
              aria-invalid={invalid || undefined}
            />
          )}
        </Field>

        <div className="rounded-2xl border border-border p-3.5">
          <Disclosure
            summary={strings.addCamera.cameraAccount}
            icon={<KeyRound className="size-4 text-fg-2" />}
            open={accountOpen}
            onOpenChange={setAccountOpen}
          >
            <p className="mt-2 text-[12.5px] leading-relaxed text-fg-2">{strings.addCamera.cameraAccountHelp}</p>
            <div className="mt-3 grid grid-cols-2 gap-3 pb-0.5">
              <Field label={strings.addCamera.username}>
                {({ id }) => (
                  <Input id={id} value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" />
                )}
              </Field>
              <Field label={strings.addCamera.password}>
                {({ id }) => (
                  <PasswordInput
                    id={id}
                    value={accountPassword}
                    onChange={(e) => setAccountPassword(e.target.value)}
                    autoComplete="new-password"
                  />
                )}
              </Field>
            </div>
          </Disclosure>
        </div>

        {(groups.data?.length ?? 0) > 0 && (
          <div className="grid gap-2">
            <span className="text-[13px] font-medium text-fg">{strings.addCamera.groupsLabel}</span>
            <div className="flex flex-wrap gap-2">
              {groups.data!.map((g) => (
                <Chip
                  key={g.id}
                  selected={groupIds.includes(g.id)}
                  onClick={() =>
                    setGroupIds((ids) => (ids.includes(g.id) ? ids.filter((x) => x !== g.id) : [...ids, g.id]))
                  }
                >
                  {g.name}
                </Chip>
              ))}
            </div>
          </div>
        )}

        {error?.code !== "third_party_compat_off" && (
          <div className="rounded-2xl bg-brand-softer p-3.5">
            <Disclosure
              summary={strings.addCamera.compatTitle}
              icon={<Info className="size-4 text-brand-text" />}
              open={compatOpen}
              onOpenChange={setCompatOpen}
            >
              <p className="mt-2 text-[12.5px] leading-relaxed text-fg-2">{strings.addCamera.compatBody}</p>
              <CompatSteps />
            </Disclosure>
          </div>
        )}
      </DialogBody>
      <DialogFooter className="border-t border-border pt-4">
        <Button variant="ghost" onClick={onBack} disabled={busy} className="mr-auto">
          {strings.common.back}
        </Button>
        <Button type="submit" variant="primary" loading={busy} disabled={lockedActive}>
          {busy
            ? strings.addCamera.submitting
            : lockedActive
              ? strings.addCamera.lockedWait(Math.max(1, lockedMins))
              : strings.addCamera.submit}
        </Button>
      </DialogFooter>
    </DialogForm>
  );
}

function CompatSteps() {
  return (
    <ol className="mt-3 grid gap-2">
      {strings.addCamera.compatSteps.map((s, i) => (
        <li key={s} className="flex items-center gap-2.5 text-[13px] text-fg">
          <span className="grid size-5 shrink-0 place-items-center rounded-full bg-brand text-[11px] font-semibold text-white">
            {i + 1}
          </span>
          {s}
        </li>
      ))}
    </ol>
  );
}

function ErrorBanner({ error, now }: { error: ApiError; now: number }) {
  const copy = describeError(error, { now });
  return (
    <div role="alert" className="flex gap-3 rounded-2xl border border-danger/15 bg-danger-soft p-3.5">
      <CircleAlert className="mt-0.5 size-[18px] shrink-0 text-danger" />
      <div className="min-w-0">
        <p className="text-sm font-semibold text-danger">{copy.title}</p>
        <p className="mt-0.5 text-[13px] leading-relaxed text-fg">
          {error.code === "third_party_compat_off" ? strings.addCamera.compatBody : copy.body}
        </p>
        {copy.hint && <p className="mt-1.5 text-xs text-fg-2">{copy.hint}</p>}
        {error.code === "third_party_compat_off" && <CompatSteps />}
      </div>
    </div>
  );
}

// --- Step 3: done -----------------------------------------------------------------------------

function DoneStep({ camera, onClose, onOpenLive }: { camera: Camera; onClose: () => void; onOpenLive: () => void }) {
  return (
    <div className="flex flex-col items-center px-8 pb-7 pt-10 text-center">
      <SuccessMark />
      <h2 className="mt-5 text-lg font-semibold tracking-[-0.01em] text-fg">{strings.addCamera.successTitle(camera.name)}</h2>
      <p className="mt-1.5 max-w-sm text-sm text-fg-2">{strings.addCamera.successBody}</p>
      <div className="mt-7 flex gap-2">
        <Button variant="ghost" onClick={onClose}>
          {strings.common.done}
        </Button>
        <Button variant="primary" onClick={onOpenLive} autoFocus>
          {strings.addCamera.openLive}
        </Button>
      </div>
    </div>
  );
}

function SuccessMark() {
  return (
    <motion.div
      initial={{ scale: 0.6, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={{ type: "spring", stiffness: 380, damping: 22 }}
      className="relative grid size-16 place-items-center rounded-full bg-success-soft"
    >
      <svg viewBox="0 0 40 40" className="size-9 text-success" aria-hidden>
        <motion.path
          d="M11 20.5l6 6 12-13"
          fill="none"
          stroke="currentColor"
          strokeWidth="3.4"
          strokeLinecap="round"
          strokeLinejoin="round"
          initial={{ pathLength: 0 }}
          animate={{ pathLength: 1 }}
          transition={{ duration: 0.45, delay: 0.15, ease: [0.2, 0, 0, 1] }}
        />
      </svg>
    </motion.div>
  );
}
