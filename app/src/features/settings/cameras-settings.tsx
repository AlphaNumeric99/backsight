import { useState, type FormEvent } from "react";
import { toast } from "sonner";
import { Check, EllipsisVertical, KeyRound, Pencil, Plus, Star, Trash2, X } from "lucide-react";
import type { ApiError, Camera, CameraGroup } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { describeError, toApiError } from "@/lib/errors";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Field, Input, PasswordInput } from "@/components/ui/input";
import { Chip } from "@/components/ui/misc";
import { ConfirmDialog, Dialog, DialogBody, DialogFooter, DialogForm } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { CameraSnapshot } from "@/components/camera-snapshot";
import { StatusPill } from "@/components/camera-status";
import { useRemoveCamera, useSaveGroups, useUpdateCamera } from "@/queries/cameras";
import { SettingsSection } from "./section";

export function CamerasSettingsSection({ cameras, groups }: { cameras: Camera[]; groups: CameraGroup[] }) {
  const [passwordFor, setPasswordFor] = useState<Camera | null>(null);
  const [removing, setRemoving] = useState<Camera | null>(null);
  const remove = useRemoveCamera();

  return (
    <SettingsSection id="cameras" title={strings.settings.sections.cameras}>
      {cameras.length === 0 ? (
        <p className="px-5 py-6 text-sm text-fg-2">{strings.settings.camerasEmpty}</p>
      ) : (
        cameras.map((c) => (
          <CameraRow
            key={c.id}
            camera={c}
            groups={groups}
            onUpdatePassword={() => setPasswordFor(c)}
            onRemove={() => setRemoving(c)}
          />
        ))
      )}

      <PasswordDialog camera={passwordFor} onClose={() => setPasswordFor(null)} />
      <ConfirmDialog
        open={removing !== null}
        onOpenChange={(o) => !o && setRemoving(null)}
        title={removing ? strings.settings.removeTitle(removing.name) : ""}
        body={strings.settings.removeBody}
        confirmLabel={strings.settings.removeCamera}
        destructive
        loading={remove.isPending}
        onConfirm={() => {
          if (!removing) return;
          const name = removing.name;
          remove.mutate(removing.id, {
            onSuccess: () => {
              toast.success(strings.settings.removed(name));
              setRemoving(null);
            },
            onError: (err) => toast.error(describeError(toApiError(err)).title, { description: describeError(toApiError(err)).body }),
          });
        }}
      />
    </SettingsSection>
  );
}

function CameraRow({
  camera,
  groups,
  onUpdatePassword,
  onRemove,
}: {
  camera: Camera;
  groups: CameraGroup[];
  onUpdatePassword: () => void;
  onRemove: () => void;
}) {
  const update = useUpdateCamera();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(camera.name);

  const save = (e?: FormEvent) => {
    e?.preventDefault();
    const next = name.trim();
    setEditing(false);
    if (!next || next === camera.name) {
      setName(camera.name);
      return;
    }
    update.mutate({ id: camera.id, req: { name: next } });
  };

  const toggleGroup = (id: string) => {
    const next = camera.groupIds.includes(id) ? camera.groupIds.filter((g) => g !== id) : [...camera.groupIds, id];
    update.mutate({ id: camera.id, req: { groupIds: next } });
  };

  return (
    <div className="flex items-start gap-4 px-5 py-4">
      <div className="aspect-video w-[88px] shrink-0 overflow-hidden rounded-lg">
        <CameraSnapshot camera={camera} showPlaceholderLabel={false} />
      </div>
      <div className="min-w-0 flex-1">
        {editing ? (
          <form onSubmit={save} className="flex max-w-sm items-center gap-1.5">
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.stopPropagation();
                  setName(camera.name);
                  setEditing(false);
                }
              }}
              onBlur={() => save()}
              autoFocus
              aria-label={strings.settings.renameLabel(camera.name)}
              className="h-8"
            />
            <IconButton label={strings.common.save} type="submit" size="icon-sm" onMouseDown={(e) => e.preventDefault()}>
              <Check />
            </IconButton>
            <IconButton
              label={strings.common.cancel}
              size="icon-sm"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                setName(camera.name);
                setEditing(false);
              }}
            >
              <X />
            </IconButton>
          </form>
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-sm font-semibold text-fg">{camera.name}</p>
            <StatusPill status={camera.status} className="h-5 px-2 text-[11px]" />
          </div>
        )}
        <p className="mt-0.5 truncate text-xs text-fg-3">
          {[camera.model, camera.host, camera.firmware].filter(Boolean).join(" · ")}
        </p>
        {groups.length > 0 && (
          <div className="mt-2.5 flex flex-wrap items-center gap-1.5" role="group" aria-label={strings.settings.cameraGroups}>
            {groups.map((g) => (
              <Chip
                key={g.id}
                selected={camera.groupIds.includes(g.id)}
                onClick={() => toggleGroup(g.id)}
                className="h-7 px-2.5 text-xs"
              >
                {g.name}
              </Chip>
            ))}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <IconButton
          label={camera.favorite ? strings.home.unfavorite : strings.home.favorite}
          aria-pressed={camera.favorite}
          onClick={() => update.mutate({ id: camera.id, req: { favorite: !camera.favorite } })}
        >
          <Star className={cn(camera.favorite && "fill-[#ffc53d] text-[#ffc53d]")} />
        </IconButton>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <span>
              <IconButton label={strings.common.more}>
                <EllipsisVertical />
              </IconButton>
            </span>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => setEditing(true)}>
              <Pencil />
              {strings.common.rename}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onUpdatePassword}>
              <KeyRound />
              {strings.settings.updatePassword}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem destructive onSelect={onRemove}>
              <Trash2 />
              {strings.settings.removeCamera}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

function PasswordDialog({ camera, onClose }: { camera: Camera | null; onClose: () => void }) {
  const update = useUpdateCamera({ silent: true });
  const [password, setPassword] = useState("");
  const [error, setError] = useState<ApiError | null>(null);
  const open = camera !== null;

  const close = () => {
    setPassword("");
    setError(null);
    update.reset();
    onClose();
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!camera || !password) return;
    setError(null);
    update.mutate(
      { id: camera.id, req: { cloudPassword: password } },
      {
        onSuccess: () => {
          toast.success(strings.settings.passwordUpdated);
          close();
        },
        onError: (err) => setError(toApiError(err)),
      },
    );
  };

  const copy = error ? describeError(error) : null;
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !update.isPending && close()}
      title={camera ? strings.settings.updatePasswordTitle(camera.name) : ""}
      description={strings.settings.updatePasswordBody}
      size="sm"
      dismissible={!update.isPending}
    >
      <DialogForm onSubmit={submit}>
        <DialogBody className="grid gap-3">
          <Field label={strings.addCamera.passwordLabel} error={copy ? `${copy.title}. ${copy.body}` : null}>
            {({ id, describedBy, invalid }) => (
              <PasswordInput
                id={id}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoFocus
                autoComplete="current-password"
                aria-describedby={describedBy}
                aria-invalid={invalid || undefined}
              />
            )}
          </Field>
        </DialogBody>
        <DialogFooter>
          <Button variant="ghost" onClick={close} disabled={update.isPending}>
            {strings.common.cancel}
          </Button>
          <Button type="submit" variant="primary" loading={update.isPending} disabled={!password}>
            {strings.common.save}
          </Button>
        </DialogFooter>
      </DialogForm>
    </Dialog>
  );
}

// --- Groups ---------------------------------------------------------------------------------

export function GroupsSettingsSection({ groups, cameras }: { groups: CameraGroup[]; cameras: Camera[] }) {
  const save = useSaveGroups();
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editName, setEditName] = useState("");

  const add = (e: FormEvent) => {
    e.preventDefault();
    const name = draft.trim();
    if (!name) return;
    const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "group";
    let id = base;
    for (let n = 2; groups.some((g) => g.id === id); n++) id = `${base}-${n}`;
    save.mutate([...groups, { id, name }]);
    setDraft("");
  };

  const rename = (id: string) => {
    const name = editName.trim();
    setEditing(null);
    if (!name) return;
    save.mutate(groups.map((g) => (g.id === id ? { ...g, name } : g)));
  };

  return (
    <SettingsSection id="groups" title={strings.settings.sections.groups} description={strings.settings.groupsHelp}>
      {groups.length === 0 && <p className="px-5 py-4 text-sm text-fg-2">{strings.settings.groupsEmpty}</p>}
      {groups.map((g) => {
        const count = cameras.filter((c) => c.groupIds.includes(g.id)).length;
        return (
          <div key={g.id} className="flex items-center gap-3 px-5 py-3">
            {editing === g.id ? (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  rename(g.id);
                }}
                className="flex flex-1 items-center gap-1.5"
              >
                <Input
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  onBlur={() => rename(g.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.stopPropagation();
                      setEditing(null);
                    }
                  }}
                  autoFocus
                  aria-label={strings.settings.renameGroup(g.name)}
                  className="h-8 max-w-xs"
                />
              </form>
            ) : (
              <p className="flex-1 text-sm font-medium text-fg">
                {g.name}
                <span className="ml-2 text-xs font-normal tabular-nums text-fg-3">{count}</span>
              </p>
            )}
            <IconButton
              label={strings.settings.renameGroup(g.name)}
              size="icon-sm"
              onClick={() => {
                setEditing(g.id);
                setEditName(g.name);
              }}
            >
              <Pencil />
            </IconButton>
            <IconButton
              label={strings.settings.deleteGroup(g.name)}
              size="icon-sm"
              variant="danger-ghost"
              onClick={() => save.mutate(groups.filter((x) => x.id !== g.id))}
            >
              <Trash2 />
            </IconButton>
          </div>
        );
      })}
      <form onSubmit={add} className="flex items-center gap-2 bg-surface-2 px-5 py-3">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={strings.settings.groupName}
          aria-label={strings.settings.groupName}
          className="h-9 max-w-xs"
        />
        <Button type="submit" variant="soft" size="sm" className="h-9" disabled={!draft.trim()}>
          <Plus />
          {strings.settings.newGroup}
        </Button>
      </form>
    </SettingsSection>
  );
}
