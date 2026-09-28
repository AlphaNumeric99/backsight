import { Link } from "@tanstack/react-router";
import { Compass } from "lucide-react";
import { strings } from "@/lib/strings";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";

export function NotFoundPage() {
  return (
    <div className="grid flex-1 place-items-center p-8">
      <EmptyState
        art={
          <div className="grid size-16 place-items-center rounded-2xl bg-brand-soft text-brand-text">
            <Compass className="size-8" strokeWidth={1.6} />
          </div>
        }
        title={strings.notFound.title}
        body={strings.notFound.body}
        actions={
          <Button asChild variant="primary">
            <Link to="/">{strings.notFound.home}</Link>
          </Button>
        }
      />
    </div>
  );
}
