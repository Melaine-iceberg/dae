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

/**
 * 折叠策略：首项始终保留，省略号占一个“单元”，余下空间从末项往前尽量多塞。
 * 返回需要折叠的索引范围 [start, end)，全都能放下时返回 null。
 *
 * @param unit 一个面包屑条目之间的固定开销（分隔符宽 + flex 间距）。
 * @param ellipsisWidth 省略号按钮自身的宽度，调用方需额外加上一个 unit。
 */
function computeHiddenRange(
  widths: readonly number[],
  availableWidth: number,
  unit: number,
  ellipsisWidth: number,
): [number, number] | null {
  const count = widths.length;
  if (count <= 2) {
    return null;
  }

  const totalWidth = widths.reduce((sum, width) => sum + width, 0) + unit * (count - 1);

  // 1px 容差吸收子像素舍入，避免恰好卡在边界时反复折叠/展开
  if (totalWidth <= availableWidth + 1) {
    return null;
  }

  let usedWidth = widths[0] + unit + ellipsisWidth;
  let start = count - 1;
  while (start > 1 && usedWidth + widths[start - 1] + unit <= availableWidth) {
    usedWidth += widths[start - 1] + unit;
    start -= 1;
  }

  return start <= 1 ? null : [1, start];
}

export function ExplorerBreadcrumbs({ breadcrumbs, onNavigate }: ExplorerBreadcrumbsProps) {
  const { t } = useTranslation("explorer");
  const containerRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLOListElement | null>(null);
  const measureRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<(HTMLLIElement | null)[]>([]);
  const separatorRef = useRef<HTMLLIElement | null>(null);
  const ellipsisTriggerRef = useRef<HTMLButtonElement | null>(null);
  // 被折叠进省略号的面包屑索引范围 [start, end)，null 表示全部展示
  const [hiddenRange, setHiddenRange] = useState<[number, number] | null>(null);

  useLayoutEffect(() => {
    const container = containerRef.current;
    const list = listRef.current;
    if (!container || !list) return;

    let disposed = false;
    // 仅在折叠区间真正变化时写状态：ResizeObserver 每帧都会回调，无谓的
    // setState 会让拖拽缩放期间的每一帧都重渲染。
    const setRange = (next: [number, number] | null) => {
      if (disposed) return;
      setHiddenRange((previous) => {
        if (previous === next) return previous;
        if (previous && next && previous[0] === next[0] && previous[1] === next[1]) {
          return previous;
        }
        return next;
      });
    };

    const compute = () => {
      const count = breadcrumbs.length;
      const widths = itemRefs.current
        .slice(0, count)
        .map((element) => element?.getBoundingClientRect().width ?? 0);
      const gap = parseFloat(getComputedStyle(list).columnGap) || 0;
      const unit = (separatorRef.current?.getBoundingClientRect().width ?? 0) + gap;
      const ellipsisWidth = (ellipsisTriggerRef.current?.getBoundingClientRect().width ?? 0) + unit;

      setRange(computeHiddenRange(widths, list.clientWidth, unit, ellipsisWidth));
    };

    compute();
    // 导航条宽度（跟随窗口）和隐藏测量层的宽度（跟随字体/目录名）任一变化都要
    // 重算，否则展开方向没有触发点。
    const observer = new ResizeObserver(compute);
    observer.observe(container);
    if (measureRef.current) {
      observer.observe(measureRef.current);
    }
    if (document.fonts) {
      void document.fonts.ready.then(compute);
    }

    return () => {
      disposed = true;
      observer.disconnect();
    };
  }, [breadcrumbs]);

  const visible = hiddenRange
    ? [breadcrumbs[0], ...breadcrumbs.slice(hiddenRange[1])]
    : breadcrumbs;
  const collapsed = hiddenRange ? breadcrumbs.slice(hiddenRange[0], hiddenRange[1]) : undefined;

  return (
    // flex-1 让导航条宽度恒等于可用宽度；否则它的宽度由内容决定，折叠后内容
    // 变窄 → 宽度变小 → 计算认为放不下 → 永远回不到展开态。
    <Breadcrumb ref={containerRef} className="relative min-w-0 flex-1">
      <BreadcrumbList ref={listRef} className="flex-nowrap">
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

      {/* 隐藏测量层：按完整路径渲染，用于测量各项自身的（不被压缩的）宽度。
         外层裁到导航条宽度，超长路径不会撑出横向滚动。 */}
      <div
        aria-hidden
        className="pointer-events-none invisible absolute inset-x-0 top-0 h-0 overflow-hidden"
      >
        <div className="flex w-max flex-nowrap" ref={measureRef}>
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
  // Catppuccin folder artwork per crumb name; drive roots and other
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
