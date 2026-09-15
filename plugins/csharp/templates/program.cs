// {{name}}, created {{date}}. Mod+Enter runs it.
using TinkerPad;

class Program
{
    static void Main()
    {
        var greeting = "Hello from C#";
        Console.WriteLine(greeting);
        new { Greeting = greeting, greeting.Length }.Dump();
    }
}
