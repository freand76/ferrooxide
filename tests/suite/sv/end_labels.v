// End labels that repeat the construct's name, also after a statement label
// (IEEE 1800-2017 9.3.4).
package pkg;
endpackage : pkg
module test;
  class C;
    function int f;
      return 1;
    endfunction : f
  endclass : C
  task t;
  endtask : t
  if (1) begin : g
  end : g
  g2 : begin
  end : g2
  initial begin : b
    C c = new;
    fork : fk
    join : fk
    l1 : begin
    end : l1
    l2 : fork
    join_none : l2
    t();
    if (c.f() == 1) $display("PASSED");
  end : b
endmodule : test
