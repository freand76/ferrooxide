// Instances of one fork branch running at once each have their own variables of the
// blocks of the automatic subroutines they call (a method's `begin int v; ... end`).
class C;
  task run(int k, output int r);
    begin
      int v = k;
      #(10 - k);
      r = v;
    end
  endtask
endclass
module t;
  C c = new;
  int got[4];
  int bad = 0;
  initial begin
    for (int j = 0; j < 4; j++)
      fork
        automatic int i = j;
        c.run(i, got[i]);
      join_none
    wait fork;
    foreach (got[i]) if (got[i] != i) bad++;
    if (bad == 0) $display("PASSED");
    else $display("FAILED: %p", got);
  end
endmodule
