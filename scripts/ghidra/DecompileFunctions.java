// Ghidra headless post-script for scripts/re.sh: decompiles every function
// whose name (as Ghidra shows it, demangled) contains the given text and
// writes the C-like output to one file. The output is for reading only and
// stays in work/re/ (never in the repo; see docs/reverse-engineering.md).
//
// Script arguments: <name text> <output file> [max functions, default 20]
// A name text of "*" matches every function (scripts/re.sh dump).
//@category OpenKF

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;

import java.io.PrintWriter;

public class DecompileFunctions extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            printerr("usage: DecompileFunctions.java <name text> <output file> [max]");
            return;
        }
        String needle = args[0];
        int max = args.length > 2 ? Integer.parseInt(args[2]) : 20;
        DecompInterface decomp = new DecompInterface();
        decomp.openProgram(currentProgram);
        int found = 0;
        try (PrintWriter out = new PrintWriter(args[1], "UTF-8")) {
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                String name = f.getName(true);
                if (!needle.equals("*") && !name.contains(needle)) {
                    continue;
                }
                found++;
                out.printf("// ===== %s @ %s (%s)%n", name, f.getEntryPoint(), currentProgram.getName());
                DecompileResults r = decomp.decompileFunction(f, 120, monitor);
                if (r.decompileCompleted()) {
                    out.println(r.getDecompiledFunction().getC());
                } else {
                    out.println("// decompile failed: " + r.getErrorMessage());
                }
                if (found >= max) {
                    out.printf("// stopped after %d functions (raise the limit to see more)%n", max);
                    break;
                }
            }
        }
        decomp.dispose();
        println("DecompileFunctions: " + found + " function(s) matching '" + needle + "' -> " + args[1]);
    }
}
