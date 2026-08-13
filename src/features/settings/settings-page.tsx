import { motion } from 'framer-motion';
import { Download, Moon, Palette, User } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Separator } from '@/components/ui/separator';
import { Switch } from '@/components/ui/switch';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Avatar, AvatarFallback } from '@/components/ui/avatar';
import { PageContainer, PageHeader } from '@/components/layout/page-header';
import { useUIStore } from '@/store/ui-store';
import { useCurrentUser } from '@/providers/auth-guard';
import { getDefaultDownloadLocation, setDefaultDownloadLocation } from '@/api/tauri';

export function SettingsPage() {
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const user = useCurrentUser();
  const [downloadLocation, setDownloadLocation] = useState<string>('');

  useEffect(() => {
    void getDefaultDownloadLocation().then((path) => setDownloadLocation(path)).catch(() => {
      setDownloadLocation('');
    });
  }, []);

  const handleChooseDownloadLocation = async () => {
    const selectedPath = await open({
      directory: true,
      multiple: false,
      title: 'Choose default download location',
    });

    if (!selectedPath) {
      return;
    }

    await setDefaultDownloadLocation(selectedPath);
    setDownloadLocation(selectedPath);
  };

  return (
    <PageContainer>
      <PageHeader title="Settings" description="Manage your account, security, and preferences." />

      <Tabs defaultValue="profile" className="gap-6">
        <TabsList className="w-fit">
          <TabsTrigger value="profile" className="gap-2">
            <User className="h-4 w-4" />
            Profile
          </TabsTrigger>
          {/* <TabsTrigger value="security" className="gap-2">
            <Lock className="h-4 w-4" />
            Security
          </TabsTrigger>
          <TabsTrigger value="notifications" className="gap-2">
            <Bell className="h-4 w-4" />
            Notifications
          </TabsTrigger> */}
          <TabsTrigger value="appearance" className="gap-2">
            <Palette className="h-4 w-4" />
            Appearance
          </TabsTrigger>
        </TabsList>

        {/* Profile */}
        <TabsContent value="profile">
          <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
            <Card className="max-w-2xl">
              <CardHeader>
                <CardTitle className="text-base">Profile</CardTitle>
                <CardDescription>Update your personal information.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <div className="flex items-center gap-4">
                  <Avatar className="h-16 w-16">
                    <AvatarFallback className="bg-muted text-sm font-medium">
                      {user.initials}
                    </AvatarFallback>
                  </Avatar>
                  <div className="space-y-1">
                    <Button variant="outline" size="sm">Change avatar</Button>
                    <p className="text-xs text-muted-foreground">PNG or JPG, max 2 MB.</p>
                  </div>
                </div>
                <Separator />
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="firstName">First name</Label>
                    <Input id="firstName" defaultValue={user.firstName} />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="lastName">Last name</Label>
                    <Input id="lastName" defaultValue={user.lastName} />
                  </div>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="email">Email</Label>
                  <Input id="email" type="email" defaultValue={user.email} />
                </div>
                {/* <div className="flex justify-end">
                  <Button size="sm">Save changes</Button>
                </div> */}
              </CardContent>
            </Card>

            <Card className="mt-6 max-w-2xl">
              <CardHeader>
                <CardTitle className="text-base">Default download location</CardTitle>
                <CardDescription>
                  Choose where transfers should be saved by default. If you leave this unset, the Windows Downloads folder is used.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex items-center gap-3 rounded-md border bg-muted/30 p-3 text-sm">
                  <Download className="h-4 w-4 text-muted-foreground" />
                  <span className="truncate">{downloadLocation || 'Downloads'}</span>
                </div>
                <div className="flex justify-end">
                  <Button variant="outline" size="sm" onClick={handleChooseDownloadLocation}>
                    Choose folder
                  </Button>
                </div>
              </CardContent>
            </Card>
          </motion.div>
        </TabsContent>

        {/* Security */}
        <TabsContent value="security">
          <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
            <Card className="max-w-2xl">
              <CardHeader>
                <CardTitle className="text-base">Security</CardTitle>
                <CardDescription>Keep your account secure.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <ToggleRow
                  title="Two-factor authentication"
                  description="Require a second factor at sign-in."
                  defaultChecked
                />
                <Separator />
                <ToggleRow
                  title="Session timeout"
                  description="Automatically sign out after 30 minutes of inactivity."
                />
                <Separator />
                <div className="space-y-2">
                  <Label htmlFor="currentPw">Change password</Label>
                  <Input id="currentPw" type="password" placeholder="Current password" />
                  <Input type="password" placeholder="New password" className="mt-2" />
                </div>
                <div className="flex justify-end">
                  <Button size="sm">Update password</Button>
                </div>
              </CardContent>
            </Card>
          </motion.div>
        </TabsContent>

        {/* Notifications */}
        <TabsContent value="notifications">
          <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
            <Card className="max-w-2xl">
              <CardHeader>
                <CardTitle className="text-base">Notifications</CardTitle>
                <CardDescription>Choose what you want to be notified about.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <ToggleRow title="Transfer completed" description="When a transfer finishes sending or receiving." defaultChecked />
                <Separator />
                <ToggleRow title="New device connected" description="When a new device joins your workspace." defaultChecked />
                <Separator />
                <ToggleRow title="Member joined" description="When an invite is accepted." />
                <Separator />
                <ToggleRow title="Weekly digest" description="A summary of activity every Monday." defaultChecked />
                <Separator />
                <ToggleRow title="Security alerts" description="Suspicious sign-in attempts." defaultChecked />
              </CardContent>
            </Card>
          </motion.div>
        </TabsContent>

        {/* Appearance */}
        <TabsContent value="appearance">
          <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
            <Card className="max-w-2xl">
              <CardHeader>
                <CardTitle className="text-base">Appearance</CardTitle>
                <CardDescription>Customize how VilSend looks on your device.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <div className="flex items-center justify-between">
                  <div className="flex items-start gap-3">
                    <Moon className="mt-0.5 h-5 w-5 text-muted-foreground" />
                    <div>
                      <p className="text-sm font-medium">Dark mode</p>
                      <p className="text-xs text-muted-foreground">
                        {theme === 'dark' ? 'Enabled' : 'Disabled'} — easier on the eyes at night.
                      </p>
                    </div>
                  </div>
                  <Switch checked={theme === 'dark'} onCheckedChange={toggleTheme} />
                </div>
              </CardContent>
            </Card>
          </motion.div>
        </TabsContent>
      </Tabs>
    </PageContainer>
  );
}

function ToggleRow({
  title,
  description,
  defaultChecked,
}: {
  title: string;
  description: string;
  defaultChecked?: boolean;
}) {
  return (
    <div className="flex items-center justify-between">
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <Switch defaultChecked={defaultChecked} />
    </div>
  );
}
