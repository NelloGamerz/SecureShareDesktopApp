import { Fragment } from 'react';
import { Link, useLocation } from 'react-router-dom';
import { ChevronRight } from 'lucide-react';
import { navItems } from '@/lib/constants';
import { cn } from '@/lib/utils';
import { useOrganizationOverview } from '@/features/organization/organization-hooks';

const routeLabels: Record<string, string> = Object.fromEntries(
  navItems.map((n) => [n.to, n.title])
);

function buildCrumbs(pathname: string, resolvedRouteLabels: Record<string, string>) {
  const segments = pathname.split('/').filter(Boolean);
  const crumbs: { label: string; to: string }[] = [];
  let acc = '';
  for (const seg of segments) {
    acc += `/${seg}`;
    const label = resolvedRouteLabels[acc];
    if (label) {
      crumbs.push({ label, to: acc });
    } else {
      // Dynamic segment (e.g. an id) — capitalize for display.
      crumbs.push({
        label: seg.charAt(0).toUpperCase() + seg.slice(1),
        to: acc,
      });
    }
  }
  return crumbs;
}

export function Breadcrumbs({ className }: { className?: string }) {
  const { pathname } = useLocation();
  const { data: organizationOverview } = useOrganizationOverview();
  const resolvedRouteLabels = {
    ...routeLabels,
    '/organization':
      organizationOverview?.organization?.type === 'INDIVIDUAL'
        ? 'Workspace'
        : 'Organization',
  };
  const crumbs = buildCrumbs(pathname, resolvedRouteLabels);

  if (crumbs.length === 0) return null;

  return (
    <nav
      aria-label="Breadcrumb"
      className={cn('flex items-center text-sm', className)}
    >
      <ol className="flex items-center gap-1.5">
        {crumbs.map((crumb, i) => {
          const isLast = i === crumbs.length - 1;
          return (
            <Fragment key={crumb.to}>
              <li>
                {isLast ? (
                  <span className="font-medium text-foreground">{crumb.label}</span>
                ) : (
                  <Link
                    to={crumb.to}
                    className="text-muted-foreground transition-colors hover:text-foreground"
                  >
                    {crumb.label}
                  </Link>
                )}
              </li>
              {!isLast && (
                <li className="text-muted-foreground/50">
                  <ChevronRight className="h-3.5 w-3.5" />
                </li>
              )}
            </Fragment>
          );
        })}
      </ol>
    </nav>
  );
}
