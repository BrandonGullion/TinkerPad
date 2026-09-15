import { dump } from "tinkerpad";

// {{name}}, created {{date}}. Mod+Enter runs it.

function main(): void {
  const greeting = "Hello from {{name}}";
  console.log(greeting);
  dump({ greeting, length: greeting.length });
}

main();
