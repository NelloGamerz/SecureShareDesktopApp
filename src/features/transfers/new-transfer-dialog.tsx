import * as React from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { stat } from "@tauri-apps/plugin-fs";
import { saveLocalTransferFile } from "@/api/tauri";

import { getDeviceInfo } from "@/services/getDeviceInfo";

import {
  AlertCircle,
  ArrowRight,
  Check,
  ChevronLeft,
  ChevronRight,
  FileUp,
  FolderUp,
  Loader2,
  Monitor,
  Send,
  X,
} from "lucide-react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

import { ScrollArea } from "@/components/ui/scroll-area";

import type { Device, OrganizationMember } from "@/features/devices/devices-api";
import { useMyDevices, useOrganizationMembers } from "@/features/devices/devices-hooks";

import { cn } from "@/lib/utils";

import { useCreateTransfer } from "./transfers-hooks";

type PickerOptions = {
  multiple: boolean;
  directory?: boolean;
};

async function selectPaths(options: PickerOptions): Promise<string[]> {
  const result = await open({
    multiple: options.multiple,
    directory: options.directory,
  });

  if (!result) {
    return [];
  }

  return Array.isArray(result) ? result : [result];
}

function displayPath(path: string) {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).at(-1) ?? path;
}

function StepRail({ currentStep }: { currentStep: 1 | 2 | 3 }) {
  const steps = ["Content", "Destination", "Review"];

  return (
    <ol
      className="
        grid
        grid-cols-3
        border-b
        bg-muted/30
      "
      aria-label="Transfer setup progress"
    >
      {steps.map((label, index) => {
        const step = (index + 1) as 1 | 2 | 3;

        const complete = step < currentStep;

        const active = step === currentStep;

        return (
          <li
            key={label}
            className={cn(
              `
              relative
              flex
              items-center
              justify-center
              gap-2
              px-2
              py-3
              text-xs
              font-medium
              sm:justify-start
              sm:px-5
              `,
              active && "bg-background text-foreground",

              !active && !complete && "text-muted-foreground",
            )}
          >
            {index > 0 && (
              <span
                className="
                  absolute
                  left-0
                  top-3
                  h-5
                  border-l
                "
              />
            )}

            <span
              className={cn(
                `
                flex
                h-5
                w-5
                items-center
                justify-center
                rounded-full
                text-[10px]
                `,

                complete && "bg-primary text-primary-foreground",

                active && "border border-primary text-primary",

                !active && !complete && "border text-muted-foreground",
              )}
            >
              {complete ? <Check className="h-3 w-3" /> : step}
            </span>

            <span className="hidden sm:inline">{label}</span>
          </li>
        );
      })}
    </ol>
  );
}

function DeviceRow({
  device,
  selected,
  onSelect,
  disabled,
}: {
  device: Device;
  selected: boolean;
  onSelect: () => void;
  disabled?: boolean;
}) {
  const available = device.status !== "offline";

  return (
    <button
      type="button"
      disabled={disabled || !available}
      onClick={onSelect}
      className={cn(
        `
        flex
        w-full
        items-center
        gap-3
        border-b
        px-3
        py-3
        text-left
        transition-colors
        last:border-b-0
        focus-visible:outline-none
        focus-visible:ring-1
        focus-visible:ring-ring
        disabled:cursor-not-allowed
        disabled:opacity-50
        `,

        selected ? "bg-primary/10" : "hover:bg-muted/60",
      )}
    >
      <span
        className={cn(
          "h-2.5 w-2.5 rounded-full",

          device.status === "online" ? "bg-emerald-500" : "bg-amber-500",
        )}
      />

      <span
        className="
          flex
          h-8
          w-8
          items-center
          justify-center
          rounded-md
          bg-muted
        "
      >
        <Monitor
          className="
            h-4
            w-4
            text-muted-foreground
          "
        />
      </span>

      <span
        className="
          min-w-0
          flex-1
        "
      >
        <span
          className="
            block
            truncate
            text-sm
            font-medium
          "
        >
          {device.deviceName}
        </span>

        <span
          className="
            block
            truncate
            text-xs
            text-muted-foreground
          "
        >
          {device.operatingSystem}
          {" · "}
          {device.region}
        </span>
      </span>

      <Badge variant="secondary" className="capitalize">
        {device.connectionType}
      </Badge>

      {selected && <Check className="h-4 w-4 text-primary" />}
    </button>
  );
}

function getMemberDisplayName(member: OrganizationMember) {
  const fullName = [member.firstName, member.lastName]
    .filter(Boolean)
    .join(" ")
    .trim();

  return member.name || fullName || "Unnamed member";
}

function MyDevicesSection({
  devices,
  selectedDeviceId,
  onSelectDevice,
  disabled,
}: {
  devices: Device[];
  selectedDeviceId?: string;
  onSelectDevice: (deviceId: string) => void;
  disabled?: boolean;
}) {
  return (
    <div className="space-y-2">
      <p className="text-sm font-semibold">My Devices</p>

      <ScrollArea className="h-40 rounded-md border">
        {devices.length === 0 ? (
          <div className="flex h-full items-center justify-center px-4 py-6 text-center text-sm text-muted-foreground">
            No personal devices available.
          </div>
        ) : (
          devices.map((device) => (
            <DeviceRow
              key={device.id}
              device={device}
              selected={selectedDeviceId === device.id}
              onSelect={() => onSelectDevice(device.id)}
              disabled={disabled}
            />
          ))
        )}
      </ScrollArea>
    </div>
  );
}

function MemberRow({
  member,
  onSelect,
  disabled,
}: {
  member: OrganizationMember;
  onSelect: () => void;
  disabled?: boolean;
}) {
  const deviceCount = member.devices.length;

  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      className={cn(
        `
        flex
        w-full
        items-center
        justify-between
        border-b
        px-3
        py-3
        text-left
        transition-colors
        last:border-b-0
        focus-visible:outline-none
        focus-visible:ring-1
        focus-visible:ring-ring
        hover:bg-muted/60
        `,
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {getMemberDisplayName(member)}
        </span>

        <span className="block truncate text-xs text-muted-foreground">
          {deviceCount} device{deviceCount === 1 ? "" : "s"}
        </span>
      </span>

      <ChevronRight className="ml-3 h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

function MemberDevicesView({
  member,
  selectedDeviceId,
  onBack,
  onSelectDevice,
  disabled,
}: {
  member: OrganizationMember;
  selectedDeviceId?: string;
  onBack: () => void;
  onSelectDevice: (deviceId: string) => void;
  disabled?: boolean;
}) {
  return (
    <div className="rounded-md border">
      <div className="flex items-center border-b px-3 py-2">
        <Button type="button" variant="ghost" size="sm" onClick={onBack} disabled={disabled}>
          <ChevronLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
      </div>

      <div className="px-3 py-2 text-sm font-medium">
        {getMemberDisplayName(member)}
      </div>

      <ScrollArea className="max-h-40">
        {member.devices.length === 0 ? (
          <div className="px-3 py-6 text-sm text-muted-foreground">
            No devices available for this member.
          </div>
        ) : (
          member.devices.map((device) => (
            <DeviceRow
              key={device.id}
              device={device}
              selected={selectedDeviceId === device.id}
              onSelect={() => onSelectDevice(device.id)}
              disabled={disabled}
            />
          ))
        )}
      </ScrollArea>
    </div>
  );
}

function MembersSection({
  members,
  selectedDeviceId,
  onSelectDevice,
  disabled,
}: {
  members: OrganizationMember[];
  selectedDeviceId?: string;
  onSelectDevice: (deviceId: string) => void;
  disabled?: boolean;
}) {
  const [activeMemberId, setActiveMemberId] = React.useState<string | null>(null);

  const activeMember = members.find((member) => member.id === activeMemberId);

  if (activeMember) {
    return (
      <MemberDevicesView
        member={activeMember}
        selectedDeviceId={selectedDeviceId}
        onBack={() => setActiveMemberId(null)}
        onSelectDevice={(deviceId) => {
          setActiveMemberId(null);
          onSelectDevice(deviceId);
        }}
        disabled={disabled}
      />
    );
  }

  return (
    <div className="space-y-2">
      <p className="text-sm font-semibold">Organization Members</p>

      <ScrollArea className="h-40 rounded-md border">
        {members.length === 0 ? (
          <div className="flex h-full items-center justify-center px-4 py-6 text-center text-sm text-muted-foreground">
            No organization members available.
          </div>
        ) : (
          members.map((member) => (
            <MemberRow
              key={member.id}
              member={member}
              onSelect={() => setActiveMemberId(member.id)}
              disabled={disabled}
            />
          ))
        )}
      </ScrollArea>
    </div>
  );
}

export function NewTransferDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [paths, setPaths] = React.useState<string[]>([]);

  const [receiverId, setReceiverId] = React.useState<string>();

  const [pickerError, setPickerError] = React.useState<string>();

  const [startError, setStartError] = React.useState<string>();
  const [currentDeviceIdentifier, setCurrentDeviceIdentifier] =
    React.useState<string>();

  const {
    data: myDevices,
    isLoading: myDevicesLoading,
    isError: myDevicesError,
  } = useMyDevices();

  const {
    data: members,
    isLoading: membersLoading,
    isError: membersError,
  } = useOrganizationMembers();

  const createTransfer = useCreateTransfer();

  const submitting = createTransfer.isPending;

  const availableMyDevices = React.useMemo(() => {
    return (myDevices ?? []).filter(
      (device) =>
        device.status !== "offline" &&
        device.deviceIdentifier !== currentDeviceIdentifier,
    );
  }, [currentDeviceIdentifier, myDevices]);

  const availableMembers = React.useMemo(() => {
    const filteredMembers = (members ?? []).map((member) => ({
      ...member,
      devices: member.devices.filter(
        (device) =>
          device.status !== "offline" &&
          device.deviceIdentifier !== currentDeviceIdentifier,
      ),
    }));

    return filteredMembers.filter((member) => member.devices.length > 0);
  }, [currentDeviceIdentifier, members]);

  const selectedDevice = React.useMemo(() => {
    const combinedDevices = [
      ...availableMyDevices,
      ...availableMembers.flatMap((member) => member.devices),
    ];

    return combinedDevices.find((device) => device.id === receiverId);
  }, [availableMembers, availableMyDevices, receiverId]);

  const selectedRecipientName = React.useMemo(() => {
    const member = availableMembers.find((member) =>
      member.devices.some((device) => device.id === receiverId),
    );

    return member ? getMemberDisplayName(member) : "My device";
  }, [availableMembers, receiverId]);

  /*
    Important:
    If files are removed while on step 2,
    automatically go back to step 1.
  */
  React.useEffect(() => {
    if (paths.length === 0 && receiverId) {
      setReceiverId(undefined);
    }
  }, [paths, receiverId]);

  React.useEffect(() => {
    const loadDeviceInfo = async () => {
      try {
        const info = await getDeviceInfo();
        setCurrentDeviceIdentifier(info.deviceIdentifier);
      } catch (error) {
        console.error("Failed to get device info", error);
      }
    };

    loadDeviceInfo();
  }, []);

  const currentStep: 1 | 2 | 3 =
    paths.length === 0 ? 1 : selectedDevice ? 3 : 2;

  const reset = React.useCallback(() => {
    setPaths([]);
    setReceiverId(undefined);
    setPickerError(undefined);
    setStartError(undefined);

    createTransfer.reset();
  }, [createTransfer]);

  const changeOpen = (nextOpen: boolean) => {
    if (submitting) {
      return;
    }

    if (!nextOpen) {
      reset();
    }

    onOpenChange(nextOpen);
  };

  const pick = async (directory = false) => {
    setPickerError(undefined);

    try {
      const selected = await selectPaths({
        multiple: true,
        directory,
      });

      if (selected.length > 0) {
        setPaths((current) => [...new Set([...current, ...selected])]);
      }
    } catch (error) {
      setPickerError(error instanceof Error ? error.message : String(error));
    }
  };

  const removePath = (path: string) => {
    setPaths((current) => current.filter((item) => item !== path));
  };

  const submit = async () => {
    if (!receiverId || paths.length === 0) {
      return;
    }

    try {
      const deviceInfo = await getDeviceInfo();

      const files = await Promise.all(
        paths.map(async (path) => {
          const fileInfo = await stat(path);

          return {
            path,
            name: displayPath(path),
            size: fileInfo.size,
          };
        }),
      );

      const response = await createTransfer.mutateAsync({
        receiverId,

        senderDeviceIdentifier: deviceInfo.deviceIdentifier,

        files: files.map((file) => ({
          name: file.name,
          size: file.size,
        })),
      });

      /*
      Save local file mapping after backend creates transfer.

      Backend stores:
      - transfer id
      - receiver
      - file metadata

      SQLite stores:
      - actual local path
      - file name
      - size
    */
      await Promise.all(
        files.map((file) =>
          saveLocalTransferFile({
            transfer_id: response.transferId,
            file_path: file.path,
            file_name: file.name,
            file_size: file.size,
          }),
        ),
      );

      changeOpen(false);
    } catch (error) {
      setStartError(error instanceof Error ? error.message : String(error));
    }
  };

  const errorMessage =
    startError ?? createTransfer.error?.message ?? pickerError;

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        className="
          flex
          max-h-[90vh]
          w-[95vw]
          max-w-2xl
          flex-col
          overflow-hidden
          p-0
        "
      >
        <DialogHeader
          className="
            shrink-0
            border-b
            px-4
            py-4
            sm:px-6
          "
        >
          <DialogTitle>New transfer</DialogTitle>

          <DialogDescription>
            Send files securely to a device in your organization.
          </DialogDescription>
        </DialogHeader>

        <StepRail currentStep={currentStep} />

        <div
          className="
            flex-1
            space-y-5
            overflow-y-auto
            px-4
            py-5
            sm:px-6
          "
        >
          <section>
            <div
              className="
                mb-3
                flex
                items-center
                justify-between
              "
            >
              <div>
                <p
                  className="
                    text-sm
                    font-semibold
                  "
                >
                  {paths.length ? "Content selected" : "1. Select content"}
                </p>

                <p
                  className="
                    text-xs
                    text-muted-foreground
                  "
                >
                  Files and folders are read locally.
                </p>
              </div>

              {paths.length > 0 && (
                <Badge variant="secondary">{paths.length} items</Badge>
              )}
            </div>

            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={submitting}
                onClick={() => pick(false)}
              >
                <FileUp className="h-4 w-4" />
                Add files
              </Button>

              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={submitting}
                onClick={() => pick(true)}
              >
                <FolderUp className="h-4 w-4" />
                Add folder
              </Button>
            </div>

            {paths.length > 0 && (
              <ScrollArea
                className="
                    mt-3
                    h-32
                    rounded-md
                    border
                  "
              >
                <ul>
                  {paths.map((path) => (
                    <li
                      key={path}
                      className="
                              flex
                              items-center
                              gap-2
                              border-b
                              px-3
                              py-2
                              text-sm
                            "
                    >
                      <FileUp
                        className="
                                h-4
                                w-4
                              "
                      />

                      <span
                        className="
                                flex-1
                                truncate
                              "
                      >
                        {displayPath(path)}
                      </span>

                      <button
                        type="button"
                        disabled={submitting}
                        onClick={() => removePath(path)}
                      >
                        <X
                          className="
                                  h-4
                                  w-4
                                "
                        />
                      </button>
                    </li>
                  ))}
                </ul>
              </ScrollArea>
            )}
          </section>

          {paths.length > 0 && (
            <section>
              <p
                className="
                    mb-3
                    text-sm
                    font-semibold
                  "
              >
                2. Choose destination
              </p>

              {(myDevicesLoading || membersLoading) && (
                <div
                  className="
                        flex
                        h-24
                        items-center
                        justify-center
                      "
                >
                  <Loader2
                    className="
                          animate-spin
                        "
                  />
                </div>
              )}

              {!myDevicesLoading && !membersLoading && (
                <div className="space-y-4">
                  <MyDevicesSection
                    devices={availableMyDevices}
                    selectedDeviceId={receiverId}
                    onSelectDevice={setReceiverId}
                    disabled={submitting}
                  />

                  <MembersSection
                    members={availableMembers}
                    selectedDeviceId={receiverId}
                    onSelectDevice={setReceiverId}
                    disabled={submitting}
                  />

                  {!myDevicesError && !membersError &&
                    availableMyDevices.length === 0 &&
                    availableMembers.length === 0 && (
                      <div className="flex h-24 items-center justify-center rounded-md border text-sm text-muted-foreground">
                        No other devices available for transfer.
                      </div>
                    )}
                </div>
              )}

              {(myDevicesError || membersError) && (
                <div className="rounded-md border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive">
                  We couldn't load device options right now. Please try again shortly.
                </div>
              )}
            </section>
          )}

          {selectedDevice && (
            <section>
              <p
                className="
                    mb-3
                    text-sm
                    font-semibold
                  "
              >
                3. Ready to send
              </p>

              <div
                className="
                    grid
                    gap-4
                    rounded-md
                    border
                    bg-muted/30
                    p-4
                    sm:grid-cols-[1fr_auto_1fr]
                    sm:items-center
                  "
              >
                <div>
                  <p
                    className="
                        text-xs
                        text-muted-foreground
                      "
                  >
                    Sending
                  </p>

                  <p
                    className="
                        truncate
                        font-semibold
                      "
                  >
                    {paths.length} file{paths.length === 1 ? "" : "s"}
                  </p>
                </div>

                <ArrowRight
                  className="
                      mx-auto
                      h-5
                      w-5
                    "
                />

                <div
                  className="
                      text-left
                      sm:text-right
                    "
                >
                  <p
                    className="
                        text-xs
                        text-muted-foreground
                      "
                  >
                    To
                  </p>

                  <p
                    className="
                        truncate
                        font-semibold
                      "
                  >
                    {selectedRecipientName}
                  </p>

                  <p
                    className="
                        mt-2
                        text-xs
                        text-muted-foreground
                      "
                  >
                    Device
                  </p>

                  <p
                    className="
                        truncate
                        font-semibold
                      "
                  >
                    {selectedDevice.deviceName}
                  </p>
                </div>
              </div>
            </section>
          )}

          {errorMessage && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />

              <AlertDescription>{errorMessage}</AlertDescription>
            </Alert>
          )}
        </div>

        <DialogFooter
          className="
            shrink-0
            border-t
            px-4
            py-4
            sm:px-6
          "
        >
          <Button
            variant="ghost"
            disabled={submitting}
            onClick={() => changeOpen(false)}
          >
            Cancel
          </Button>

          <Button
            disabled={!receiverId || paths.length === 0 || submitting}
            onClick={submit}
          >
            {submitting ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                Preparing...
              </>
            ) : (
              <>
                <Send className="h-4 w-4" />
                Send
                <ChevronRight className="h-4 w-4" />
              </>
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
