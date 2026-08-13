import * as React from "react";
import { motion } from "framer-motion";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  // Download,
  MoreHorizontal,
  Plus,
  Check,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { useTransferRequestStore } from "@/store/transfer-request-store";
import {
  useTransferControls,
  useTransferProgressListener,
} from "./transfers-hooks";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import { PageContainer, PageHeader } from "@/components/layout/page-header";
import { NewTransferDialog } from "./new-transfer-dialog";
import { useTransfers, useTransferAction } from "./transfers-hooks";
import { Progress } from "@/components/ui/progress";

// const statusVariant: Record<
//   string,
//   "default" | "secondary" | "success" | "destructive"
// > = {
//   COMPLETED: "success",
//   IN_PROGRESS: "secondary",
//   PENDING: "default",
//   FAILED: "destructive",
//   CANCELLED: "destructive",
// };

const statusVariant: Record<
  string,
  "default" | "secondary" | "success" | "destructive"
> = {
  PENDING: "default",
  CONNECTING: "secondary",
  WAITING_SENDER: "secondary",
  HANDSHAKING: "secondary",
  IN_PROGRESS: "secondary",
  UPLOADING: "secondary",
  DOWNLOADING: "secondary",
  COMPLETED: "success",
  FAILED: "destructive",
  CANCELLED: "destructive",
};

function formatFileSize(size: number) {
  if (size < 1024) {
    return `${size} B`;
  }

  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(1)} KB`;
  }

  if (size < 1024 * 1024 * 1024) {
    return `${(size / (1024 * 1024)).toFixed(1)} MB`;
  }

  return `${(size / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

const preparingStatus: Record<string, string> = {
  CONNECTING: "Connecting...",
  WAITING_SENDER: "Waiting for sender...",
  HANDSHAKING: "Establishing secure connection...",
};

function IndeterminateProgress() {
  return (
    <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-muted">
      <div className="absolute h-full w-1/3 animate-indeterminate rounded-full bg-primary" />
    </div>
  );
}

export function TransfersPage() {
  // const [isNewTransferOpen, setIsNewTransferOpen] = React.useState(false);

  // const { data: transfers = [], isLoading, isError } = useTransfers();

  const [isNewTransferOpen, setIsNewTransferOpen] = React.useState(false);

  const requests = useTransferRequestStore((state) => state.requests);

  const uniqueRequests = React.useMemo(() => {
    return Array.from(
      new Map(requests.map((request) => [request.id, request])).values(),
    );
  }, [requests]);

  // const removeRequest = useTransferRequestStore((state) => state.removeRequest);
  useTransferProgressListener();

  const { data: transfers = [], isLoading, isError } = useTransfers();

  const transferAction = useTransferAction();
  const controls = useTransferControls();

  const handleTransferAction = (
    transferId: string,
    action: "ACCEPT" | "REJECT" | "CANCEL",
  ) => {
    transferAction.mutate({
      transferId,
      action,
    });
  };

  return (
    <PageContainer>
      <PageHeader
        title="Transfers"
        description="Track every file moving in and out of your organization."
        actions={
          <Button size="sm" onClick={() => setIsNewTransferOpen(true)}>
            <Plus className="h-4 w-4" />
            New transfer
          </Button>
        }
      />

      {uniqueRequests.length > 0 && (
        <Card className="mb-4">
          <CardContent className="space-y-3 p-4">
            <h3 className="text-sm font-semibold">
              Incoming transfer requests
            </h3>

            {uniqueRequests.map((request) => (
              <div
                key={request.id}
                className="flex items-center justify-between rounded-lg border p-3"
              >
                <div className="flex items-center gap-3">
                  <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-muted">
                    <ArrowDownToLine className="h-4 w-4" />
                  </div>

                  <div>
                    <p className="text-sm font-medium">{request.file_name}</p>

                    <p className="text-xs text-muted-foreground">
                      {formatFileSize(request.file_size)}
                    </p>
                  </div>
                </div>

                <div className="flex gap-2">
                  <Button
                    size="sm"
                    onClick={() => {
                      console.log("Accept:", request.id);
                      handleTransferAction(request.id, "ACCEPT");

                      // removeRequest(request.transfer_id);
                    }}
                  >
                    <Check className="mr-1 h-4 w-4" />
                    Accept
                  </Button>

                  <Button
                    size="sm"
                    variant="destructive"
                    onClick={() => {
                      console.log("Reject transfer", request.transfer_id);
                      handleTransferAction(request.id, "REJECT");

                      // TODO:
                      // invoke reject_transfer

                      // removeRequest(request.transfer_id);
                    }}
                  >
                    <X className="mr-1 h-4 w-4" />
                    Reject
                  </Button>
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[360px]">File</TableHead>

                <TableHead className="hidden md:table-cell">
                  Direction
                </TableHead>

                <TableHead className="hidden lg:table-cell">Device</TableHead>

                <TableHead className="hidden lg:table-cell">Network</TableHead>

                <TableHead>Status</TableHead>

                <TableHead className="hidden sm:table-cell">Date</TableHead>

                <TableHead className="w-10" />
              </TableRow>
            </TableHeader>

            <TableBody>
              {isLoading && (
                <TableRow>
                  <TableCell colSpan={7} className="py-10 text-center">
                    Loading transfers...
                  </TableCell>
                </TableRow>
              )}

              {isError && (
                <TableRow>
                  <TableCell
                    colSpan={7}
                    className="py-10 text-center text-destructive"
                  >
                    Failed to load transfers
                  </TableCell>
                </TableRow>
              )}

              {!isLoading &&
                !isError &&
                transfers.map((t, i) => {
                  const progress =
                    t.totalBytes > 0
                      ? (t.uploadedBytes / t.totalBytes) * 100
                      : 0;

                  const isPreparing =
                    t.status === "CONNECTING" ||
                    t.status === "WAITING_SENDER" ||
                    t.status === "HANDSHAKING";

                  // const showProgress =
                  //   t.status === "UPLOADING" ||
                  //   t.status === "DOWNLOADING" ||
                  //   t.status === "IN_PROGRESS";

                  const showProgress =
                    isPreparing ||
                    t.status === "UPLOADING" ||
                    t.status === "DOWNLOADING" ||
                    t.status === "IN_PROGRESS";

                  return (
                    <motion.tr
                      key={t.id}
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      transition={{ delay: i * 0.03 }}
                      className="group"
                    >
                      <TableCell>
                        <div className="flex items-start gap-3">
                          <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted">
                            {t.sentByMe ? (
                              <ArrowUpFromLine className="h-4 w-4 text-muted-foreground" />
                            ) : (
                              <ArrowDownToLine className="h-4 w-4 text-muted-foreground" />
                            )}
                          </div>

                          <div className="min-w-0 flex-1">
                            <p className="truncate text-sm font-medium">
                              {t.fileName}
                            </p>

                            <p className="text-xs text-muted-foreground">
                              {formatFileSize(t.fileSize)}
                            </p>

                            {/* {showProgress && (
                              <div className="mt-3 space-y-1.5">
                                <Progress value={progress} className="h-1.5" />

                                <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                                  <span>
                                    {formatFileSize(t.uploadedBytes)} /{" "}
                                    {formatFileSize(t.totalBytes)}
                                  </span>

                                  <span>{Math.round(progress)}%</span>
                                </div>
                              </div>
                            )} */}
                            {showProgress && (
                              <div className="mt-3 space-y-1.5">
                                {isPreparing ? (
                                  <>
                                    <IndeterminateProgress />

                                    <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                                      <span>
                                        {preparingStatus[t.status] ??
                                          "Preparing transfer..."}
                                      </span>

                                      <span className="animate-pulse">•••</span>
                                    </div>
                                  </>
                                ) : (
                                  <>
                                    <Progress
                                      value={progress}
                                      className="h-1.5"
                                    />

                                    <div className="flex items-center justify-between text-[11px] text-muted-foreground">
                                      <span>
                                        {formatFileSize(t.uploadedBytes)} /{" "}
                                        {formatFileSize(t.totalBytes)}
                                      </span>

                                      <span>{Math.round(progress)}%</span>
                                    </div>
                                  </>
                                )}
                              </div>
                            )}
                          </div>
                        </div>
                      </TableCell>

                      <TableCell className="hidden md:table-cell">
                        <span className="text-sm text-muted-foreground">
                          {t.sentByMe ? "Sent" : "Received"}
                        </span>
                      </TableCell>

                      <TableCell className="hidden lg:table-cell">
                        <span className="text-sm text-muted-foreground">
                          {t.sentByMe
                            ? `To: ${t.receiverDeviceName}`
                            : `From: ${t.senderDeviceName}`}
                        </span>
                      </TableCell>

                      <TableCell className="hidden lg:table-cell">
                        <span className="text-sm text-muted-foreground">
                          {t.networkType}
                        </span>
                      </TableCell>

                      <TableCell>
                        <Badge variant={statusVariant[t.status] ?? "default"}>
                          {t.status}
                        </Badge>
                      </TableCell>

                      <TableCell className="hidden sm:table-cell">
                        <span className="text-sm text-muted-foreground">
                          {new Date(t.createdAt).toLocaleString()}
                        </span>
                      </TableCell>

                      <TableCell>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 opacity-0 transition-opacity group-hover:opacity-100"
                            >
                              <MoreHorizontal className="h-4 w-4" />
                            </Button>
                          </DropdownMenuTrigger>

                          <DropdownMenuContent align="end">
                            {/* <DropdownMenuItem>
                              <Download className="mr-2 h-4 w-4" />
                              Download
                            </DropdownMenuItem> */}

                            {/* {!t.sentByMe && t.status === "PENDING" && (
                              <>
                                <DropdownMenuItem>
                                  <Check className="mr-2 h-4 w-4 text-green-600" />
                                  Accept
                                </DropdownMenuItem>

                                <DropdownMenuItem className="text-destructive">
                                  <X className="mr-2 h-4 w-4" />
                                  Reject
                                </DropdownMenuItem>
                              </>
                            )} */}
                            {t.senderDeviceId != t.receiverDeviceId &&
                              t.status === "PENDING" && (
                                <>
                                  <DropdownMenuItem
                                    onClick={() => {
                                      console.log("Accept transfer:", t.id);

                                      handleTransferAction(t.id, "ACCEPT");
                                    }}
                                  >
                                    <Check className="mr-2 h-4 w-4 text-green-600" />
                                    Accept
                                  </DropdownMenuItem>

                                  <DropdownMenuItem
                                    className="text-destructive"
                                    onClick={() => {
                                      console.log("Reject transfer:", t.id);

                                      handleTransferAction(t.id, "REJECT");
                                    }}
                                  >
                                    <X className="mr-2 h-4 w-4" />
                                    Reject
                                  </DropdownMenuItem>
                                </>
                              )}

                            {t.status === "UPLOADING" && (
                              <DropdownMenuItem
                                onClick={() =>
                                  controls.mutate({
                                    transferId: t.id,
                                    action: "PAUSE",
                                  })
                                }
                              >
                                Pause
                              </DropdownMenuItem>
                            )}

                            {t.status === "PAUSED" && (
                              <DropdownMenuItem
                                onClick={() =>
                                  controls.mutate({
                                    transferId: t.id,
                                    action: "RESUME",
                                  })
                                }
                              >
                                Resume
                              </DropdownMenuItem>
                            )}

                            {/* <DropdownMenuItem
                              className="text-destructive"
                              onClick={() =>
                                controls.mutate({
                                  transferId: t.id,
                                  action: "CANCEL",
                                })
                              }
                            >
                              Cancel
                            </DropdownMenuItem> */}

                            {t.sentByMe && t.status === "PENDING" && (
                              <DropdownMenuItem
                                className="text-destructive"
                                onClick={() =>
                                  handleTransferAction(t.id, "CANCEL")
                                }
                              >
                                Cancel
                              </DropdownMenuItem>
                            )}

                            {t.status === "FAILED" && (
                              <DropdownMenuItem>Retry</DropdownMenuItem>
                            )}
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </motion.tr>
                  );
                })}

              {!isLoading && !isError && transfers.length === 0 && (
                <TableRow>
                  <TableCell
                    colSpan={7}
                    className="py-10 text-center text-muted-foreground"
                  >
                    No transfers found
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <NewTransferDialog
        open={isNewTransferOpen}
        onOpenChange={setIsNewTransferOpen}
      />
    </PageContainer>
  );
}
