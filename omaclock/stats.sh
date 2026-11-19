#!/bin/bash
# One snapshot. All output is numeric; the disk path stays a quoted argument.
# Usage: bash stats.sh <showCpu> <showMemory> <showDisk> <diskPath>
export LC_ALL=C

if [[ $1 == true ]] && read -r kind user nice system idle iowait irq softirq steal rest < /proc/stat; then
  # guest/guest_nice (in rest) are already included in user/nice.
  printf 'cpu %s %s\n' "$((user + nice + system + idle + iowait + irq + softirq + steal))" "$((idle + iowait))"
fi

if [[ $2 == true ]]; then
  total= available=
  while read -r key value unit; do
    case $key in
      MemTotal:) total=$value ;;
      MemAvailable:) available=$value ;;
    esac
    [[ -n $total && -n $available ]] && break
  done < /proc/meminfo
  [[ -n $total && -n $available ]] && printf 'memory %s %s\n' "$total" "$available"
fi

if [[ $3 == true ]]; then
  # Bound a slow/unavailable mount so it cannot accumulate polling processes.
  # Explicit columns also handle paths containing spaces, unlike parsing df -P.
  while read -r total used available percent; do
    [[ $total =~ ^[0-9]+$ ]] || continue
    printf 'disk %s %s %s %s\n' "$total" "$used" "$available" "${percent%\%}"
  done < <(timeout --kill-after=1s 2s df -k --output=size,used,avail,pcent -- "$4" 2>/dev/null)
fi
exit 0
