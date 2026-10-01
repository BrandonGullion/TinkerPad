# The table `tinkerpad plugin check` expects (docs/PLUGINS.md §8.1).
$rows = @(
  [pscustomobject]@{ Name = 'ada'; N = 1 }
  [pscustomobject]@{ Name = 'grace'; N = 2 }
)

$rows | Dump -Title 'check'
