import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { MotionConfig } from "motion/react";
import "@fontsource-variable/inter";
import "./styles/globals.css";
import { applyTheme, readCachedTheme } from "@/lib/theme";
import { createQueryClient } from "@/queries/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import { router } from "@/app/router";

// Apply the last-used theme before the first render so there's no flash.
applyTheme(readCachedTheme());

const queryClient = createQueryClient();

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <MotionConfig reducedMotion="user">
        <TooltipProvider>
          <RouterProvider router={router} />
        </TooltipProvider>
      </MotionConfig>
    </QueryClientProvider>
  </StrictMode>,
);
