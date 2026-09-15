// {{name}}, created {{date}}. Mod+Enter runs it.
// NuGet packages go at the very top, e.g. #:package Humanizer@2.14.1
using TinkerPad;

new[] { new { Name = "Ada", Born = 1815 }, new { Name = "Grace", Born = 1906 } }.Dump("people");
