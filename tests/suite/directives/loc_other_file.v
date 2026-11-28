// expected
// flags: loc_other_file_pkg.sv
// Messages from code in another file of the command line (a class, a specialization of a
// class, a function in a file that file includes) name the file and line they come from.
module m;
  import loc_pkg::*;
  c o;
  d #(3) p;
  initial begin
    o = new;
    p = new;
    o.hello();
    p.hello();
    inc_hello();
    $info("the module, line 15");
  end
endmodule
