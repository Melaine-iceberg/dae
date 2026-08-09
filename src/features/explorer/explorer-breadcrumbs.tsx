import { Fragment } from "react";
import { FolderIcon } from "lucide-react";

import {
  Breadcrumb,
  BreadcrumbEllipsis,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import type { Breadcrumb as BreadcrumbData } from "./types";

interface ExplorerBreadcrumbsProps {
  breadcrumbs: BreadcrumbData[];
  onNavigate: (breadcrumb: BreadcrumbData) => void;
}

type VisibleBreadcrumb =
  | { type: "breadcrumb"; breadcrumb: BreadcrumbData }
  | { type: "ellipsis"; breadcrumbs: BreadcrumbData[] };

const MAX_VISIBLE_BREADCRUMBS = 4;

export function ExplorerBreadcrumbs({ breadcrumbs, onNavigate }: ExplorerBreadcrumbsProps) {
  const visibleBreadcrumbs = collapseBreadcrumbs(breadcrumbs);
  const currentPath = breadcrumbs.at(-1)?.path;

  return (
    <Breadcrumb className="min-w-0">
      <BreadcrumbList className="flex-nowrap">
        {visibleBreadcrumbs.map((item, index) => {
          const key = item.type === "ellipsis" ? "collapsed-path" : item.breadcrumb.path;

          return (
            <Fragment key={key}>
              {index > 0 && <BreadcrumbSeparator />}
              <BreadcrumbItem className="min-w-0">
                {item.type === "ellipsis" ? (
                  <DropdownMenu>
                    <DropdownMenuTrigger
                      render={
                        <Button
                          aria-label="显示完整路径"
                          size="icon-sm"
                          title="显示完整路径"
                          type="button"
                          variant="ghost"
                        />
                      }
                    >
                      <BreadcrumbEllipsis />
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="start" className="min-w-48 max-w-80">
                      <DropdownMenuGroup>
                        {item.breadcrumbs.map((breadcrumb) => (
                          <DropdownMenuItem
                            key={breadcrumb.path}
                            onClick={() => onNavigate(breadcrumb)}
                            title={breadcrumb.path}
                          >
                            <FolderIcon />
                            <span className="truncate">{breadcrumb.name}</span>
                          </DropdownMenuItem>
                        ))}
                      </DropdownMenuGroup>
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : item.breadcrumb.path === currentPath ? (
                  <BreadcrumbPage className="max-w-64 truncate">
                    {item.breadcrumb.name}
                  </BreadcrumbPage>
                ) : (
                  <BreadcrumbLink
                    className="max-w-40 truncate"
                    render={<button type="button" onClick={() => onNavigate(item.breadcrumb)} />}
                  >
                    {item.breadcrumb.name}
                  </BreadcrumbLink>
                )}
              </BreadcrumbItem>
            </Fragment>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}

function collapseBreadcrumbs(breadcrumbs: BreadcrumbData[]): VisibleBreadcrumb[] {
  if (breadcrumbs.length <= MAX_VISIBLE_BREADCRUMBS) {
    return breadcrumbs.map((breadcrumb) => ({
      type: "breadcrumb",
      breadcrumb,
    }));
  }

  return [
    { type: "breadcrumb", breadcrumb: breadcrumbs[0] },
    { type: "ellipsis", breadcrumbs: breadcrumbs.slice(1, -2) },
    ...breadcrumbs.slice(-2).map((breadcrumb): VisibleBreadcrumb => ({
      type: "breadcrumb",
      breadcrumb,
    })),
  ];
}
