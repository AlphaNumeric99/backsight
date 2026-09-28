import { useEffect, useRef, type ReactNode } from "react";
import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { motion } from "motion/react";
import { Cctv, Download, LayoutGrid, PanelLeftClose, PanelLeftOpen, Settings, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { applyTheme } from "@/lib/theme";
import { Wordmark } from "@/components/brand";
import { StatusDot } from "@/components/camera-status";
import { Tooltip } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/toaster";
import { useMediaQuery } from "@/hooks/use-media-query";
import { useCameras } from "@/queries/cameras";
import { useActiveExportCount } from "@/queries/exports";
import { useSettings } from "@/queries/settings";
import { ApiEventBridge } from "@/queries/bridge";
import { useUiStore } from "@/state/ui";
import { AddCameraDialog } from "@/features/add-camera/add-camera-dialog";

interface NavItem {
  to: "/" | "/multiview" | "/downloads" | "/settings";
  label: string;
  icon: LucideIcon;
  isActive: (path: string) => boolean;
  badge?: number;
}

function useSidebarCollapsed(): [boolean, (collapsed: boolean) => void] {
  const pref = useUiStore((s) => s.sidebarCollapsed);
  const set = useUiStore((s) => s.setSidebarCollapsed);
  const narrow = useMediaQuery("(max-width: 1099px)");
  return [pref ?? narrow, (collapsed) => set(collapsed)];
}

function Sidebar() {
  const [collapsed, setCollapsed] = useSidebarCollapsed();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const activeJobs = useActiveExportCount();
  const cameras = useCameras();
  const online = cameras.data?.filter((c) => c.status.state === "online").length ?? 0;
  const total = cameras.data?.length ?? 0;

  const main: NavItem[] = [
    { to: "/", label: strings.nav.cameras, icon: Cctv, isActive: (p) => p === "/" || p.startsWith("/cameras") },
    { to: "/multiview", label: strings.nav.multiview, icon: LayoutGrid, isActive: (p) => p.startsWith("/multiview") },
    {
      to: "/downloads",
      label: strings.nav.downloads,
      icon: Download,
      isActive: (p) => p.startsWith("/downloads"),
      badge: activeJobs,
    },
  ];
  const settings: NavItem = {
    to: "/settings",
    label: strings.nav.settings,
    icon: Settings,
    isActive: (p) => p.startsWith("/settings"),
  };

  return (
    <aside
      className={cn(
        "relative z-20 flex h-full shrink-0 flex-col overflow-hidden whitespace-nowrap border-r border-border bg-sidebar transition-[width] duration-(--dur-slow) ease-out-expo",
        collapsed ? "w-[76px]" : "w-[232px]",
      )}
    >
      <div className={cn("flex h-[68px] items-center", collapsed ? "justify-center" : "px-5")}>
        <Link to="/" aria-label={strings.app.name} className="rounded-xl">
          <Wordmark collapsed={collapsed} />
        </Link>
      </div>

      <nav aria-label={strings.nav.label} className="flex flex-1 flex-col gap-1 px-3 pt-2">
        {main.map((item) => (
          <NavLink key={item.to} item={item} active={item.isActive(path)} collapsed={collapsed} />
        ))}
        <div className="flex-1" />
        {!collapsed && total > 0 && (
          <div className="mb-2 flex items-center gap-2 rounded-xl px-3 py-2 text-xs text-fg-2">
            <StatusDot tone={online === total ? "success" : online > 0 ? "warning" : "neutral"} />
            {strings.nav.onlineSummary(online, total)}
          </div>
        )}
        <NavLink item={settings} active={settings.isActive(path)} collapsed={collapsed} />
      </nav>

      <div className={cn("flex px-3 pb-4 pt-2", collapsed ? "justify-center" : "justify-start")}>
        <Tooltip content={collapsed ? strings.nav.expand : strings.nav.collapse} side="right">
          <button
            type="button"
            onClick={() => setCollapsed(!collapsed)}
            aria-label={collapsed ? strings.nav.expand : strings.nav.collapse}
            aria-expanded={!collapsed}
            className="grid size-9 place-items-center rounded-xl text-fg-3 transition-colors hover:bg-hover hover:text-fg"
          >
            {collapsed ? <PanelLeftOpen className="size-[18px]" /> : <PanelLeftClose className="size-[18px]" />}
          </button>
        </Tooltip>
      </div>
    </aside>
  );
}

function NavLink({ item, active, collapsed }: { item: NavItem; active: boolean; collapsed: boolean }) {
  const Icon = item.icon;
  const badge = item.badge ?? 0;
  const link = (
    <Link
      to={item.to}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? (badge > 0 ? `${item.label}, ${strings.nav.activeJobs(badge)}` : item.label) : undefined}
      className={cn(
        "group relative flex h-10 items-center gap-3 rounded-xl text-[14px] font-medium outline-offset-0 transition-colors duration-(--dur-fast)",
        collapsed ? "justify-center px-0" : "px-3",
        active ? "text-brand-text" : "text-fg-2 hover:bg-hover hover:text-fg",
      )}
    >
      {active && (
        <motion.span
          layoutId="nav-active"
          className="absolute inset-0 rounded-xl bg-brand-soft"
          transition={transitions.spring}
          aria-hidden
        />
      )}
      <span className="relative">
        <Icon className="size-[19px]" strokeWidth={active ? 2.1 : 1.9} aria-hidden />
        {collapsed && badge > 0 && (
          <span className="absolute -right-2 -top-1.5 grid h-4 min-w-4 place-items-center rounded-full bg-brand px-1 text-[10px] font-semibold leading-none text-white ring-2 ring-sidebar">
            {badge}
          </span>
        )}
      </span>
      {!collapsed && <span className="relative flex-1 truncate">{item.label}</span>}
      {!collapsed && badge > 0 && (
        <span
          className="relative grid h-5 min-w-5 place-items-center rounded-full bg-brand px-1.5 text-[11px] font-semibold tabular-nums text-white"
          aria-label={strings.nav.activeJobs(badge)}
        >
          {badge}
        </span>
      )}
    </Link>
  );
  return collapsed ? (
    <Tooltip content={item.label} side="right">
      {link}
    </Tooltip>
  ) : (
    link
  );
}

/** Applies the saved theme once settings load (the cached one is applied before first paint). */
function ThemeSync() {
  const { data } = useSettings();
  useEffect(() => {
    if (data?.theme) applyTheme(data.theme);
  }, [data?.theme]);
  return null;
}

function SkipLink() {
  return (
    <button
      type="button"
      onClick={() => document.getElementById("main")?.focus()}
      className="sr-only z-[80] rounded-full bg-brand px-4 py-2 text-sm font-medium text-white focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
    >
      {strings.app.skipToContent}
    </button>
  );
}

export function RootLayout() {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const mainRef = useRef<HTMLElement>(null);
  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0 });
  }, [path]);

  return (
    <div className="flex h-full bg-background text-fg">
      <SkipLink />
      <Sidebar />
      <main
        id="main"
        ref={mainRef}
        tabIndex={-1}
        className="relative min-w-0 flex-1 overflow-y-auto overflow-x-hidden outline-none"
      >
        <PageTransition key={path}>
          <Outlet />
        </PageTransition>
      </main>
      <ApiEventBridge />
      <ThemeSync />
      <AddCameraDialog />
      <Toaster />
    </div>
  );
}

function PageTransition({ children }: { children: ReactNode }) {
  return (
    <motion.div
      className="flex min-h-full flex-col"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0, transition: transitions.slow }}
    >
      {children}
    </motion.div>
  );
}
