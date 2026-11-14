module t;
  reg [1:0] s; reg [3:0] y;
  task run;
    begin
      unique case (s)
        2'd0: y = 1;
        2'd1, 2'd0: y = 2;
      endcase
      priority case (s)
        2'd0: y = 3;
        2'd1: y = 4;
      endcase
      unique0 case (s)
        2'd1: y = 5;
      endcase
      unique casez (s)
        2'b1?: y = 6;
        2'b?1: y = 7;
      endcase
      $display("%0t s=%b y=%0d", $time, s, y);
    end
  endtask
  initial begin
    s = 0; #1 run; s = 1; #1 run; s = 2; #1 run; s = 3; #1 run; s = 2'bx; #1 run;
  end
endmodule
