import { useCallback, useState } from "react";
import { Modal } from "../../components/Modal";
import { SchedulePanel } from "../backupRestore/SchedulePanel";
import type { ScheduleConfig } from "../../ipc/commands";

interface EditScheduleModalProps {
  open: boolean;
  jobId: string;
  schedule: ScheduleConfig;
  onClose: () => void;
  onSave: (jobId: string, config: ScheduleConfig) => Promise<void>;
}

export function EditScheduleModal({ open, jobId, schedule, onClose, onSave }: EditScheduleModalProps) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleChange = useCallback(
    async (config: ScheduleConfig | null) => {
      setSaving(true);
      setError(null);
      try {
        if (!config) {
          // Disabling schedule: keep same cron but set enabled=false
          await onSave(jobId, { ...schedule, enabled: false, nextRunAt: null });
        } else {
          await onSave(jobId, config);
        }
        onClose();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setSaving(false);
      }
    },
    [jobId, schedule, onSave, onClose],
  );

  return (
    <Modal
      open={open}
      title="Edit schedule"
      onClose={onClose}
      width={420}
      footer={
        <div className="modal__footer" style={{ justifyContent: "space-between" }}>
          <button className="btn btn--ghost" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          {error && <span className="job-meta--errors" style={{ fontSize: "var(--font-size-sm)" }}>{error}</span>}
        </div>
      }
    >
      <SchedulePanel
        value={schedule.enabled ? schedule : null}
        onChange={handleChange}
      />
    </Modal>
  );
}
