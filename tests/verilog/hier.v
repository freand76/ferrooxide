// Hierarchical names: reads, writes, parameters, event controls and task calls into other
// instances; $sformat.
module leaf #(parameter W = 4, parameter real T = 2.5) (input clk);
  reg [W-1:0] cnt = 0;
  reg [7:0] note = 8'h11;
  always @(posedge clk) cnt <= cnt + 1;
  task report(input integer k);
    $display("%m: report %0d cnt=%0d note=%h T=%f", k, cnt, note, T);
  endtask
endmodule

module mid(input clk);
  leaf #(.W(6)) l(clk);
endmodule

module tb;
  reg clk = 0;
  mid a(clk);
  leaf b(clk);
  reg [8*24-1:0] s;
  always @(posedge a.l.cnt[1]) $display("%0t: a.l.cnt bit1 rose, cnt=%0d", $time, a.l.cnt);
  initial begin
    repeat (6) #5 clk = ~clk;
    $display("a.l.cnt=%0d b.cnt=%0d W=%0d", a.l.cnt, b.cnt, a.l.W);
    a.l.note = 8'hab;
    a.l.report(1);
    b.report(2);
    $sformat(s, "cnt%0d_%h", b.cnt, a.l.note);
    $display("[%0s]", s);
  end
endmodule
