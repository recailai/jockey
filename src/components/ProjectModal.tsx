import { createSignal, Show } from "solid-js";
import { Folder, FolderPlus, Sparkles } from "lucide-solid";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Alert, Button, Dialog, DialogBody, DialogContent, DialogFooter, FormField, Input } from "./ui";
import type { Project } from "../lib/tauriApi";

type ProjectModalProps = {
  open: boolean;
  onClose: () => void;
  onCreateProject: (name: string, rootPath: string) => Promise<Project | void>;
};

export default function ProjectModal(props: ProjectModalProps) {
  const [name, setName] = createSignal("");
  const [rootPath, setRootPath] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const handlePathChange = (val: string) => {
    setRootPath(val);
    if (!name() && val.trim()) {
      const parts = val.trim().replace(/\\/g, "/").split("/");
      const last = parts.filter(Boolean).pop();
      if (last) setName(last);
    }
  };

  const handleBrowse = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Select Project Directory",
      });
      if (typeof selected === "string" && selected.trim()) {
        handlePathChange(selected.trim());
      }
    } catch {
      // ignore
    }
  };

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    const pName = name().trim();
    const pPath = rootPath().trim();
    if (!pName || !pPath) {
      setError("Please fill in both project directory path and name");
      return;
    }

    try {
      setIsSubmitting(true);
      setError(null);
      await props.onCreateProject(pName, pPath);
      setName("");
      setRootPath("");
      props.onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={props.open} onOpenChange={(open) => { if (!open) props.onClose(); }}>
      <DialogContent
        title="Add Project"
        description="Import your local repository and CLI agent sessions"
        icon={<FolderPlus size={20} class="text-[var(--ui-accent)]" />}
      >
        <form onSubmit={handleSubmit}>
          <DialogBody class="space-y-4">
            <Show when={error()}>
              <Alert tone="danger" onClose={() => setError(null)}>
                {error()}
              </Alert>
            </Show>

            <FormField label="Project Directory Path" required>
              <div class="flex items-center gap-2">
                <div class="relative flex-1 flex items-center">
                  <Folder size={14} class="absolute left-3 theme-muted pointer-events-none" />
                  <Input
                    placeholder="/Users/username/workspace/my-project"
                    value={rootPath()}
                    onInput={(e) => handlePathChange(e.currentTarget.value)}
                    class="pl-9 text-xs"
                    autofocus
                  />
                </div>
                <Button
                  type="button"
                  variant="secondary"
                  size="md"
                  onClick={handleBrowse}
                  class="shrink-0 text-xs"
                >
                  Browse…
                </Button>
              </div>
            </FormField>

            <FormField label="Project Display Name" required>
              <Input
                placeholder="my-project"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                class="text-xs"
              />
            </FormField>
          </DialogBody>

          <DialogFooter
            cancelText="Cancel"
            confirmText={isSubmitting() ? "Adding…" : "Add & Import"}
            confirmLoading={isSubmitting()}
            onCancel={props.onClose}
          >
            <Button
              variant="secondary"
              size="md"
              type="button"
              onClick={props.onClose}
              class="text-xs"
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              type="submit"
              disabled={isSubmitting()}
              class="inline-flex items-center gap-1.5 text-xs"
            >
              <Sparkles size={13} />
              {isSubmitting() ? "Adding…" : "Add & Import"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
