import { Command } from "../command";

import { runCommand } from "./run";
import { versionCommand } from "./version";

const runHelp = (args: string[]) => {
  if (args.length === 0 || args[0] === "help") {
    helpCommand.help();
    return;
  }

  if (args[0] === runCommand.name) {
    runCommand.help();
    return;
  } else if (args[0] === versionCommand.name) {
    versionCommand.help();
    return;
  }

  console.log(`Unrecognized subcommand: ${args[0]}`);
  helpCommand.help();
};

const command = new Command({
  name: "help",
  description: "Show the usage of the passed subcommand",
  runCallback: runHelp,
  getHelpBody: () => {
    return `USAGE:

$ wasmer-js help [SUBCOMMAND]

ARGUMENTS:

[SUBCOMMAND] - The subcommand we want to see the help message for 
    
    The available subcommands (other than help) are:
    ${runCommand.name} - ${runCommand.description}
    ${versionCommand.name} - ${versionCommand.description}`;
  }
});

export const helpCommand = command;
