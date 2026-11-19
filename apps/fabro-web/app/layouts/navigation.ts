import {
  BellAlertIcon,
  ClockIcon,
  Cog6ToothIcon,
  PlayIcon,
  SparklesIcon,
  Squares2X2Icon,
} from "@heroicons/react/24/outline";

export const navigation = [
  { name: "Automations", href: "/automations", icon: ClockIcon },
  { name: "Runs", href: "/runs", icon: PlayIcon },
  { name: "Work", href: "/work", icon: Squares2X2Icon },
  { name: "Observations", href: "/observations", icon: BellAlertIcon },
  // The global supervisor chat (ADR-0046, work#1452) — beside work and
  // observations in the main menu.
  { name: "Ask Fabro", href: "/ask", icon: SparklesIcon },
  { name: "Settings", href: "/settings", icon: Cog6ToothIcon },
];
