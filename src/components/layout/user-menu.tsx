import { useClerk, UserButton } from '@clerk/clerk-react';
import { LogOut, Settings, User } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import {
  Avatar,
  AvatarFallback,
  AvatarImage,
} from '@/components/ui/avatar';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useCurrentUser } from '@/providers/auth-guard';
import { isClerkConfigured } from '@/lib/env';
import { toast } from 'sonner';

export function UserMenu() {
  const user = useCurrentUser();
  const navigate = useNavigate();
  useClerk();

  // When Clerk is configured, use Clerk's UserButton for full profile management.
  if (isClerkConfigured) {
    return (
      <div className="flex items-center">
        <UserButton
          afterSignOutUrl="/sign-in"
          appearance={{
            elements: { userButtonAvatarBox: 'h-8 w-8' },
          }}
        />
      </div>
    );
  }

  // Preview-mode menu.
  const handleSignOut = () => {
    toast.info('Signed out (preview mode).');
    navigate('/sign-in');
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button className="flex items-center gap-2 rounded-md p-1 transition-colors hover:bg-accent">
          <Avatar className="h-8 w-8">
            <AvatarImage src={user.imageUrl} alt={user.name} />
            <AvatarFallback className="bg-muted text-xs font-medium">
              {user.initials}
            </AvatarFallback>
          </Avatar>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="font-normal">
          <div className="flex flex-col">
            <span className="text-sm font-medium">{user.name}</span>
            <span className="truncate text-xs text-muted-foreground">
              {user.email}
            </span>
          </div>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => navigate('/settings')}>
          <User className="h-4 w-4" />
          Profile
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => navigate('/settings')}>
          <Settings className="h-4 w-4" />
          Settings
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={handleSignOut}>
          <LogOut className="h-4 w-4" />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// import { useClerk } from "@clerk/clerk-react";
// import { LogOut, Settings, User } from "lucide-react";
// import { useNavigate } from "react-router-dom";
// import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
// import {
//   DropdownMenu,
//   DropdownMenuContent,
//   DropdownMenuItem,
//   DropdownMenuLabel,
//   DropdownMenuSeparator,
//   DropdownMenuTrigger,
// } from "@/components/ui/dropdown-menu";
// import { useCurrentUser } from "@/providers/auth-guard";
// import { isClerkConfigured } from "@/lib/env";
// import { toast } from "sonner";

// import { stopCloudflared, stopTauriWebSocket } from "@/api/tauri";

// export function UserMenu() {
//   const user = useCurrentUser();
//   const navigate = useNavigate();
//   const clerk = useClerk();

//   const handleSignOut = async () => {
//     try {
//       // Stop desktop services first
//       await stopTauriWebSocket();
//       await stopCloudflared();

//       if (isClerkConfigured) {
//         await clerk.signOut({
//           redirectUrl: "/sign-in",
//         });
//       } else {
//         toast.info("Signed out (preview mode).");
//         navigate("/sign-in");
//       }
//     } catch (error) {
//       console.error("Sign out failed:", error);
//       toast.error("Failed to sign out");
//     }
//   };

//   return (
//     <DropdownMenu>
//       <DropdownMenuTrigger asChild>
//         <button className="flex items-center gap-2 rounded-md p-1 transition-colors hover:bg-accent">
//           <Avatar className="h-8 w-8">
//             <AvatarImage src={user.imageUrl} alt={user.name} />
//             <AvatarFallback className="bg-muted text-xs font-medium">
//               {user.initials}
//             </AvatarFallback>
//           </Avatar>
//         </button>
//       </DropdownMenuTrigger>

//       <DropdownMenuContent align="end" className="w-56">
//         <DropdownMenuLabel className="font-normal">
//           <div className="flex flex-col">
//             <span className="text-sm font-medium">{user.name}</span>
//             <span className="truncate text-xs text-muted-foreground">
//               {user.email}
//             </span>
//           </div>
//         </DropdownMenuLabel>

//         <DropdownMenuSeparator />

//         <DropdownMenuItem onClick={() => navigate("/settings")}>
//           <User className="h-4 w-4" />
//           Profile
//         </DropdownMenuItem>

//         <DropdownMenuItem onClick={() => navigate("/settings")}>
//           <Settings className="h-4 w-4" />
//           Settings
//         </DropdownMenuItem>

//         <DropdownMenuSeparator />

//         <DropdownMenuItem onClick={handleSignOut}>
//           <LogOut className="h-4 w-4" />
//           Sign out
//         </DropdownMenuItem>
//       </DropdownMenuContent>
//     </DropdownMenu>
//   );
// }

// import { useClerk } from "@clerk/clerk-react";
// import { LogOut, Settings, User } from "lucide-react";
// import { useNavigate } from "react-router-dom";
// import { toast } from "sonner";

// import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";

// import {
//   DropdownMenu,
//   DropdownMenuContent,
//   DropdownMenuItem,
//   DropdownMenuLabel,
//   DropdownMenuSeparator,
//   DropdownMenuTrigger,
// } from "@/components/ui/dropdown-menu";

// import { useCurrentUser } from "@/providers/auth-guard";
// import { isClerkConfigured } from "@/lib/env";

// import { stopCloudflared, stopTauriWebSocket } from "@/api/tauri";

// export function UserMenu() {
//   const user = useCurrentUser();
//   const navigate = useNavigate();
//   const clerk = useClerk();

//   const handleSignOut = async () => {
//     try {
//       // Stop desktop services first
//       await stopTauriWebSocket();
//       await stopCloudflared();

//       if (isClerkConfigured) {
//         await clerk.signOut({
//           redirectUrl: "/sign-in",
//         });
//       } else {
//         toast.info("Signed out (preview mode).");
//         navigate("/sign-in");
//       }
//     } catch (error) {
//       console.error("Sign out failed:", error);
//       toast.error("Failed to sign out");
//     }
//   };

//   return (
//     <DropdownMenu>
//       <DropdownMenuTrigger asChild>
//         <button className="flex items-center gap-2 rounded-md p-1 transition-colors hover:bg-accent">
//           <Avatar className="h-8 w-8">
//             <AvatarImage src={user.imageUrl} alt={user.name} />

//             <AvatarFallback className="bg-muted text-xs font-medium">
//               {user.initials}
//             </AvatarFallback>
//           </Avatar>
//         </button>
//       </DropdownMenuTrigger>

//       <DropdownMenuContent align="end" className="w-56">
//         <DropdownMenuLabel className="font-normal">
//           <div className="flex flex-col">
//             <span className="text-sm font-medium">{user.name}</span>

//             <span className="truncate text-xs text-muted-foreground">
//               {user.email}
//             </span>
//           </div>
//         </DropdownMenuLabel>

//         <DropdownMenuSeparator />

//         <DropdownMenuItem onClick={() => navigate("/settings")}>
//           <User className="h-4 w-4" />
//           Profile
//         </DropdownMenuItem>

//         <DropdownMenuItem onClick={() => navigate("/settings")}>
//           <Settings className="h-4 w-4" />
//           Settings
//         </DropdownMenuItem>

//         <DropdownMenuSeparator />

//         <DropdownMenuItem onClick={handleSignOut}>
//           <LogOut className="h-4 w-4" />
//           Sign out
//         </DropdownMenuItem>
//       </DropdownMenuContent>
//     </DropdownMenu>
//   );
// }
