import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

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

import { getFolderPresentation } from "./file-icons";
import type { Breadcrumb as BreadcrumbData } from "./types";

interface ExplorerBreadcrumbsProps {
  breadcrumbs: BreadcrumbData[];
  onNavigate: (breadcrumb: BreadcrumbData) => void;
}

export function ExplorerBreadcrumbs({ breadcrumbs, onNavigate }: ExplorerBreadcrumbsProps) {
  const { t } = useTranslation("explorer");
  const containerRef = useRef<HTMLElement | null>(null);
  const itemRefs = useRef<(HTMLLIElement | null)[]>([]);
  const separatorRef = useRef<HTMLLIElement | null>(null);
  const ellipsisTriggerRef = useRef<HTMLButtonElement | null>(null);
  // 被折叠进省略号的面包屑索引范围 [start, end)，null 表示全部展示
  const [hiddenRange, setHiddenRange] = useState<[number, number] | null>(null);

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const compute = () => {
      const list = container.querySelector<HTMLElement>("[data-slot='breadcrumb-list']");
      if (!list) return;

      const count = breadcrumbs.length;
      const widths = itemRefs.current
        .slice(0, count)
        .map((element) => element?.getBoundingClientRect().width ?? 0);
      const gap = parseFloat(getComputedStyle(list).columnGap) || 0;
      const separatorWidth = separatorRef.current?.getBoundingClientRect().width ?? 0;

      const totalWidth =
        widths.reduce((sum, width) => sum + width, 0) +
        (separatorWidth + gap) * Math.max(count - 1, 0);

      if (count <= 2 || totalWidth <= list.clientWidth + 1) {
        setHiddenRange(null);
        return;
      }

      const ellipsisWidth =
        (ellipsisTriggerRef.current?.getBoundingClientRect().width ?? 0) + separatorWidth + gap;

      // 始终保留首项与尽量多的末尾项，中间折叠为省略号
      let usedWidth = widths[0] + separatorWidth + gap + ellipsisWidth;
      let start = count - 1;
      while (
        start > 1 &&
        usedWidth + widths[start - 1] + separatorWidth + gap <= list.clientWidth
      ) {
        usedWidth += widths[start - 1] + separatorWidth + gap;
        start -= 1;
      }

      setHiddenRange(start <= 1 ? null : [1, start]);
    };

    compute();
    const observer = new ResizeObserver(compute);
    observer.observe(container);
    return () => observer.disconnect();
  }, [breadcrumbs]);

  const visible = hiddenRange
    ? [breadcrumbs[0], ...breadcrumbs.slice(hiddenRange[1])]
    : breadcrumbs;
  const collapsed = hiddenRange ? breadcrumbs.slice(hiddenRange[0], hiddenRange[1]) : undefined;

  return (
    <Breadcrumb ref={containerRef} className="min-w-0">
      <BreadcrumbList className="flex-nowrap">
        {visible.map((breadcrumb, index) => {
          const nodes: ReactNode[] = [];
          if (index > 0) nodes.push(<BreadcrumbSeparator key="sep" />);
          nodes.push(
            <BreadcrumbItem key={breadcrumb.path} className="min-w-0">
              <CrumbContent
                breadcrumb={breadcrumb}
                isCurrent={breadcrumb.path === breadcrumbs.at(-1)?.path}
                onNavigate={onNavigate}
              />
            </BreadcrumbItem>,
          );
          if (collapsed && index === 0) {
            nodes.push(
              <BreadcrumbSeparator key="sep-collapsed" />,
              <BreadcrumbItem key="collapsed-path" className="min-w-0">
                <DropdownMenu>
                  <DropdownMenuTrigger
                    render={
                      <Button
                        aria-label={t("breadcrumbs.showFullPath")}
                        size="icon-sm"
                        title={t("breadcrumbs.showFullPath")}
                        type="button"
                        variant="ghost"
                        onClick={(event) => event.stopPropagation()}
                        onPointerDown={(event) => event.stopPropagation()}
                      />
                    }
                  >
                    <BreadcrumbEllipsis />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start" className="min-w-48 max-w-80">
                    <DropdownMenuGroup>
                      {collapsed.map((item) => {
                        const ItemIcon = getFolderPresentation(item.name).icon;
                        return (
                          <DropdownMenuItem
                            key={item.path}
                            onClick={() => onNavigate(item)}
                            title={item.path}
                          >
                            <ItemIcon className="size-4 shrink-0" />
                            <span className="truncate">{item.name}</span>
                          </DropdownMenuItem>
                        );
                      })}
                    </DropdownMenuGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
              </BreadcrumbItem>,
            );
          }
          return <Fragment key={breadcrumb.path}>{nodes}</Fragment>;
        })}
      </BreadcrumbList>

      {/* 隐藏测量层：按完整路径渲染，用于测量各项实际宽度 */}
      <div aria-hidden className="invisible absolute flex-nowrap">
        <BreadcrumbList className="flex-nowrap">
          {breadcrumbs.map((breadcrumb, index) => (
            <Fragment key={breadcrumb.path}>
              {index > 0 && (
                <BreadcrumbSeparator
                  ref={(element) => {
                    separatorRef.current = element;
                  }}
                />
              )}
              <BreadcrumbItem
                className="min-w-0"
                ref={(element) => {
                  itemRefs.current[index] = element;
                }}
              >
                <CrumbContent
                  breadcrumb={breadcrumb}
                  isCurrent={breadcrumb.path === breadcrumbs.at(-1)?.path}
                  plain
                />
              </BreadcrumbItem>
            </Fragment>
          ))}
        </BreadcrumbList>
        <Button
          ref={ellipsisTriggerRef}
          size="icon-sm"
          tabIndex={-1}
          type="button"
          variant="ghost"
        >
          <BreadcrumbEllipsis />
        </Button>
      </div>
    </Breadcrumb>
  );
}

function CrumbContent({
  breadcrumb,
  isCurrent,
  plain,
  onNavigate,
}: {
  breadcrumb: BreadcrumbData;
  isCurrent: boolean;
  plain?: boolean;
  onNavigate?: (breadcrumb: BreadcrumbData) => void;
}) {
  // Material Icon Theme folder artwork per crumb name; drive roots and other
  // unmapped names fall back to the theme's generic folder.
  const CrumbIcon = getFolderPresentation(breadcrumb.name).icon;
  const icon = <CrumbIcon className="size-3.5 shrink-0" />;

  if (plain) {
    return (
      <span className={`flex items-center gap-1 ${isCurrent ? "max-w-64" : "max-w-40"}`}>
        {icon}
        <span className="block truncate">{breadcrumb.name}</span>
      </span>
    );
  }
  if (isCurrent) {
    return (
      <BreadcrumbPage className="flex max-w-64 items-center gap-1">
        {icon}
        <span className="truncate">{breadcrumb.name}</span>
      </BreadcrumbPage>
    );
  }
  return (
    <BreadcrumbLink
      className="flex max-w-40 items-center gap-1"
      render={
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onNavigate?.(breadcrumb);
          }}
        />
      }
    >
      {icon}
      <span className="truncate">{breadcrumb.name}</span>
    </BreadcrumbLink>
  );
}
