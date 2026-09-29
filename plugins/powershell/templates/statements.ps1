# {{name}}, created {{date}}. Mod+Enter runs it.

$people = @(
  [pscustomobject]@{ Name = 'Ada'; Born = 1815 }
  [pscustomobject]@{ Name = 'Grace'; Born = 1906 }
)

$people | Dump -Title 'people'

"$($people.Count) people"
