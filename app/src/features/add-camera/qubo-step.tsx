import { useEffect, useState, type FormEvent } from "react";
import { useMutation } from "@tanstack/react-query";
import { Cctv, ChevronRight } from "lucide-react";
import { api, type Camera, type CameraAccount, type QuboCloudDevice } from "@/ipc";
import { strings } from "@/lib/strings";
import { toApiError } from "@/lib/errors";
import { cn } from "@/lib/utils";
import { DialogBody, DialogFooter, DialogForm } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Field, Input, PasswordInput } from "@/components/ui/input";
import { Badge, Chip } from "@/components/ui/misc";
import { useAddQuboCamera, useGroups } from "@/queries/cameras";

/** Account sign-in precedes discovery for cloud cameras, unlike a LAN scan. */
export function QuboStep({ onBack, onAdded, onBusy }: {
  onBack: () => void;
  onAdded: (camera: Camera) => void;
  onBusy: (busy: boolean) => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [devices, setDevices] = useState<QuboCloudDevice[] | null>(null);
  const [selected, setSelected] = useState<QuboCloudDevice | null>(null);
  const [name, setName] = useState("");
  const [groupIds, setGroupIds] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const groups = useGroups();
  const signIn = useMutation({ mutationFn: (account: CameraAccount) => api.listQuboDevices(account) });
  const add = useAddQuboCamera();
  const busy = signIn.isPending || add.isPending;

  useEffect(() => {
    onBusy(busy);
    return () => onBusy(false);
  }, [busy, onBusy]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      if (!devices) {
        setDevices(await signIn.mutateAsync({ username: username.trim(), password }));
        setPassword("");
      } else if (selected) {
        onAdded(await add.mutateAsync({ deviceUuid: selected.deviceUuid, name: name.trim() || undefined, groupIds }));
      }
    } catch (err) {
      // Qubo errors already carry account-specific copy from the backend.
      setError(toApiError(err).message);
    } finally {
      signIn.reset();
    }
  };

  return (
    <DialogForm onSubmit={submit}>
      <DialogBody className="grid gap-4 pt-3">
        {error && (
          <p role="alert" className="rounded-2xl border border-danger/15 bg-danger-soft p-3.5 text-sm text-danger">
            {error}
          </p>
        )}
        {!devices ? (
          <>
            <Field label={strings.qubo.email}>
              {({ id }) => (
                <Input
                  id={id}
                  type="email"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  autoComplete="username"
                  autoFocus
                  required
                  disabled={busy}
                />
              )}
            </Field>
            <Field label={strings.qubo.password} help={strings.qubo.passwordHelp}>
              {({ id, describedBy }) => (
                <PasswordInput
                  id={id}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="current-password"
                  aria-describedby={describedBy}
                  required
                  disabled={busy}
                />
              )}
            </Field>
          </>
        ) : (
          <>
            <p className="text-sm font-medium text-fg">{strings.qubo.choose}</p>
            {devices.length === 0 && <p className="text-sm text-fg-2">{strings.qubo.empty}</p>}
            <div className="grid max-h-[240px] gap-1 overflow-y-auto">
              {devices.map((device) => (
                <button
                  key={device.deviceUuid}
                  type="button"
                  disabled={device.alreadyAdded || busy}
                  aria-pressed={selected?.deviceUuid === device.deviceUuid}
                  onClick={() => {
                    setSelected(device);
                    setName("");
                  }}
                  className={cn(
                    "flex items-center gap-3 rounded-xl px-3 py-2.5 text-left disabled:opacity-50",
                    selected?.deviceUuid === device.deviceUuid ? "bg-brand-soft" : "hover:bg-hover",
                  )}
                >
                  <Cctv className="size-5 shrink-0 text-brand-text" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{device.name || device.model}</span>
                    <span className="block text-xs text-fg-3">{device.model}</span>
                  </span>
                  {device.alreadyAdded ? (
                    <Badge tone="neutral" size="sm">{strings.addCamera.alreadyAdded}</Badge>
                  ) : (
                    <ChevronRight className="size-4 text-fg-3" />
                  )}
                </button>
              ))}
            </div>
            {selected && (
              <Field label={strings.addCamera.nameLabel} optional>
                {({ id }) => (
                  <Input id={id} value={name} onChange={(e) => setName(e.target.value)} placeholder={selected.name} disabled={busy} />
                )}
              </Field>
            )}
            {selected && !!groups.data?.length && (
              <div className="flex flex-wrap gap-2" role="group" aria-label={strings.addCamera.groupsLabel}>
                {groups.data.map((group) => (
                  <Chip
                    key={group.id}
                    selected={groupIds.includes(group.id)}
                    onClick={() => setGroupIds((ids) => ids.includes(group.id) ? ids.filter((id) => id !== group.id) : [...ids, group.id])}
                  >
                    {group.name}
                  </Chip>
                ))}
              </div>
            )}
            <p className="text-xs text-fg-3">{strings.qubo.liveOnly}</p>
          </>
        )}
      </DialogBody>
      <DialogFooter className="border-t border-border pt-4">
        <Button variant="ghost" onClick={onBack} disabled={busy} className="mr-auto">
          {strings.common.back}
        </Button>
        <Button type="submit" variant="primary" loading={busy} disabled={devices ? !selected : !username.trim() || !password}>
          {devices ? strings.addCamera.submit : strings.qubo.signIn}
        </Button>
      </DialogFooter>
    </DialogForm>
  );
}
