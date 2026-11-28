// the package of loc_other_file.v, in a file of its own (given with `// flags:`)
package loc_pkg;
  class c;
    function void hello();
      $info("the package's file, line 5");
    endfunction
  endclass
  class d #(int N = 1);
    function void hello();
      $info("a specialization, N = %0d, line 10", N);
    endfunction
  endclass
`include "loc_other_file.svh"
endpackage
